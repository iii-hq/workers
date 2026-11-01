//! Per-frame interception: parse each JSON text frame, decide per `type`
//! whether to forward (possibly rewritten), answer it out-of-band, or drop it,
//! then re-serialize. The proxy never invents a wire format — it speaks the
//! iii worker protocol verbatim.
//!
//! It forwards only the frame types it knows ([`DOWNSTREAM_TYPES`],
//! [`UPSTREAM_TYPES`]) and drops the rest. The engine behind the proxy runs
//! with no RBAC session of its own, so a frame type the proxy has not learned
//! to gate is a frame nothing gates: protocol additions are closed until the
//! proxy learns them. Of the protocol features either side declares, it keeps
//! the ones it enforces end to end ([`SUPPORTED_FEATURES`]). The frame types the
//! iii protocol removed ([`REMOVED_FRAMES`]) are dropped with one `warn` per
//! connection and frame type, naming what replaces each, as the engine does.
//!
//! Observers are gated from both sides:
//!
//! - **Observing.** An `observe` runs the operator's observer registration
//!   hook, then needs the session to be allowed to call both its source and
//!   its target (its own registrations are exempt). `engine::` targets are
//!   refused, and so are sources whose calls carry what the session's
//!   discovery view hides. `engine::observers::list|get` are filtered like the
//!   other discovery functions, and `engine::observers::dispose` reaches only
//!   the observers this connection created.
//! - **Hosting.** The frames a host exchanges about the observers of its
//!   functions (`functionregistered`, `observeradded`, `observerremoved`,
//!   `observerack`, `invocationsobserved`, `localinvocationstats`) are
//!   forwarded for the functions this connection registered, with the session
//!   prefix rewritten both ways. A call through an observer
//!   (`activation.observer_id`) is forwarded only for an observer of a
//!   function this connection hosts; the engine checks the rest.
//! - **Call chains.** The `caller` an SDK asserts on a call or a report is
//!   kept only when it names a call the engine routed to this connection that
//!   is still unanswered, and the proxy then writes it from that call: a
//!   worker cannot claim another worker's call chain.
//!
//! Frames are parsed into [`serde_json::Value`] (not a typed `Message` enum)
//! so unknown fields and unknown frame types survive a rewrite byte-for-byte —
//! the proxy only ever mutates the specific keys it must.
//!
//! The interceptor holds the connection's [`ProxySession`] (derived once at
//! the upgrade) and the [`WorkerConfig`] snapshot captured at the upgrade —
//! tuning changes apply to the *next* connection; an in-flight connection
//! keeps the boundaries it was admitted under (spec: *Hot reload*).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use crate::config::WorkerConfig;
use crate::engine_overrides::{self, CatalogCache, OverrideOutcome};
use crate::rbac::{self, ProxySession, DEFAULT_NAMESPACE};

/// Worker → engine frame types the proxy forwards (after gating the ones it
/// rewrites). Any other type is dropped.
pub const DOWNSTREAM_TYPES: &[&str] = &[
    "registerfunction",
    "unregisterfunction",
    "invokefunction",
    "invocationresult",
    "observe",
    "unobserve",
    "observerack",
    "invocationsobserved",
    "localinvocationstats",
    "registerservice",
    "reattach",
    "ping",
    "pong",
];

/// Engine → worker frame types the proxy forwards. Any other type is dropped.
pub const UPSTREAM_TYPES: &[&str] = &[
    "workerregistered",
    "registrationrejected",
    "invokefunction",
    "invocationresult",
    "functionregistered",
    "observeradded",
    "observerremoved",
    "observeresult",
    "observerstatechanged",
    "error",
    "ping",
    "pong",
];

/// Protocol features (`features` negotiation) the proxy enforces end to end.
/// They are kept in what the worker declares on `engine::workers::register`
/// and in the engine's `workerregistered`; every other feature is removed.
/// `large-messages.v1` is not among them: the proxy forwards text frames only.
///
/// `local-dispatch.v1` lets the SDK run its own functions in-process once
/// `functionregistered` says `local_dispatch: true`, which the proxy decides
/// per function (see [`Interceptor::on_function_registered`]): an in-process
/// call never reaches the middleware.
pub const SUPPORTED_FEATURES: &[&str] = &["observe.v1", "observe.v2", "local-dispatch.v1"];

/// The frame types the iii protocol removed (0.25's binding messages), with
/// what replaces each (the engine's `REMOVED_MESSAGES`). A worker that sends
/// one gets one `warn` per connection and frame type, and the frame is
/// dropped. The engine sends none of them, so upstream they are dropped as
/// unknown types.
const REMOVED_FRAMES: [(&str, &str); 5] = [
    (
        "registertriggertype",
        "register an observable function instead (`args_format` for its args, `private: true` \
         for an event source)",
    ),
    (
        "unregistertriggertype",
        "unregister the observable function that replaces the type instead",
    ),
    (
        "registertrigger",
        "observe the function that replaces the type with iii.observe(source, target) instead",
    ),
    ("unregistertrigger", "dispose the observer instead"),
    (
        "triggerregistrationresult",
        "acknowledge observers with `observerack` instead",
    ),
];

/// The engine's observer builtins (`engine::observers::list|get|dispose`).
const OBSERVER_FUNCTIONS_PREFIX: &str = "engine::observers::";

/// Disposes an observer by id: gated to the observers this connection
/// created (the engine lets a caller without a session dispose any).
const DISPOSE_OBSERVER: &str = "engine::observers::dispose";

/// Engine events whose calls list functions, workers or observers: what the
/// discovery overrides filter. Observing one would deliver the engine's
/// unfiltered view, so they are refused as sources.
const UNFILTERED_SOURCES: &[&str] = &[
    "engine::on-functions-changed",
    "engine::on-workers-changed",
    "engine::on-observers-changed",
];

/// `RegistrationRejected.code` for a namespace the session may not declare.
const NAMESPACE_FORBIDDEN: &str = "NAMESPACE_FORBIDDEN";

/// Observe error code for an `observe` frame the engine could not decode.
const INVALID_ARGS: &str = "INVALID_ARGS";

/// How long a call routed through the middleware may run. The SDK default
/// (30 s) would cut the client's longer calls short while the work goes on;
/// the client keeps its own timeout.
const MIDDLEWARE_TIMEOUT_MS: u64 = 610_000;

/// What the downstream→engine pump should do with a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DownstreamAction {
    /// Forward this (possibly rewritten) text frame to the engine.
    Forward(String),
    /// Answer the downstream worker directly; do **not** touch the engine
    /// (deny synthesis, middleware result).
    ReplyToClient(String),
    /// Send the first frame to the engine, then answer the downstream worker
    /// with the second: a refused `observe`'s `unobserve`, then its refusal
    /// ([`Interceptor::refuse_observe`]).
    ForwardThenReply(String, String),
    /// Silently drop (denied `RegisterFunction` — the engine does the same —
    /// and any frame type the proxy does not forward).
    Drop,
    /// Answer the downstream worker with these frames, then close the
    /// connection (a namespace the session may not declare).
    Close(Vec<String>),
}

/// What the engine→downstream pump should do with a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamAction {
    Forward(String),
    /// Forward these frames, in order: an `observeresult`, then the
    /// refusals of the observes under its id that the connection sent after
    /// it ([`Requested::Refused`]).
    ForwardAll(Vec<String>),
    /// Send the first frame to the engine, then forward these: an engine
    /// refusal's `unobserve` (see the `observeresult` arm of
    /// [`Interceptor::handle_upstream`]), then the frames.
    ReplyThenForwardAll(String, Vec<String>),
    /// A frame type the proxy does not forward.
    Drop,
}

/// `(namespace, function_id)`: the key a function lives under in the engine.
type FnKey = (String, String);

/// The observers a connection deals with, from both sides.
#[derive(Default)]
struct ObserverBook {
    /// Observers of the functions this connection hosts, as the engine
    /// announced them (`functionregistered`, `observeradded`,
    /// `observerremoved`): observer id → the observed function. The
    /// connection may call through these.
    hosted: HashMap<String, FnKey>,
    /// The observes this connection sent under each id that have no answer
    /// yet, oldest first. The engine answers a connection's observes one
    /// each, in the order it reads them, and the SDK gives each answer to the
    /// oldest observe under the id with none: a worker that disposes an
    /// observer observes again under its id at once, so an id can have
    /// several. An answer ends its entry, a refusal included: the engine
    /// stores nothing for a refused observe, and the next answer under the id
    /// is the next entry's.
    requested: HashMap<String, VecDeque<Requested>>,
    /// Observers this connection created and the engine accepted, with the
    /// same value.
    created: HashMap<String, bool>,
}

/// An `observe` of this connection that has no answer yet.
enum Requested {
    /// Forwarded to the engine.
    Forwarded {
        /// Whether its activations carry the `Observed` envelope.
        envelope: bool,
        /// An `unobserve` of the id followed it to the engine. The engine
        /// reads that `unobserve` after this `observe`, so it disposes the
        /// observer this `observe` creates (or ignores it, for a refused
        /// one): an acceptance makes nothing this connection's.
        unobserved: bool,
    },
    /// Refused by the proxy while an earlier observe under the id waits for
    /// the engine's answer: the refusal, which goes out right after that
    /// answer, so the worker hears the answers in the order it sent the
    /// observes.
    Refused(String),
}

/// A call the engine routed to this connection, until the connection answers
/// it.
struct InboundCall {
    /// The function called, as the engine knows it (prefixed where
    /// applicable).
    function_id: String,
    /// The caller the engine forwarded the call with.
    caller: Option<Value>,
}

/// Per-connection interceptor state, shared (behind `Arc`) by both pump
/// directions.
pub struct Interceptor {
    /// Control connection — used to invoke the operator's auth/middleware/hook
    /// functions and to drive the discovery cache.
    pub iii: Arc<IIIClient>,
    /// Config snapshot captured at the upgrade (rbac, middleware, leak knob).
    pub config: Arc<WorkerConfig>,
    /// Shared discovery cache (the function catalog) over the control
    /// connection.
    pub catalog: Arc<CatalogCache>,
    /// Boundaries derived for this connection by the auth function.
    pub session: Arc<ProxySession>,
    /// Engine ids (prefixed where applicable) this session registered on this
    /// connection — the own-vs-foreign discriminator for calls and for an
    /// observer's source and target.
    registered_ids: Mutex<HashSet<String>>,
    /// The engine ids among `registered_ids` registered `private: true`. Only
    /// their host may call them, and the middleware runs on the control
    /// connection, so this connection's calls to them skip the middleware,
    /// and `functionregistered` keeps offering them `local_dispatch`.
    private_ids: Mutex<HashSet<String>>,
    /// The namespace this connection declared on `engine::workers::register`
    /// (`None` until it does, which the engine treats as `default`). Its own
    /// registrations live there, so an id is "own" only in this namespace.
    connection_namespace: Mutex<Option<String>>,
    /// `invocation_id → engine:: discovery fn` for results to rewrite.
    pending_overrides: Mutex<HashMap<Uuid, String>>,
    /// The observers this connection hosts and the ones it created.
    observers: Mutex<ObserverBook>,
    /// Calls the engine routed to this connection that it has not answered,
    /// by `invocation_id`: what a `caller` the worker asserts must name.
    inbound: Mutex<HashMap<String, InboundCall>>,
    /// The removed frame types this connection was warned about.
    warned_removed: Mutex<HashSet<&'static str>>,
    /// Out-of-band channel for synthesized replies (text frames) produced by
    /// spawned work — the middleware path. The downstream→engine pump must not
    /// block on the middleware call: the middleware may invoke a function owned
    /// by *this* connection, whose dispatched result has to flow back through
    /// the same pump (head-of-line deadlock if the pump is parked). So
    /// middleware runs in a spawned task and its result is delivered here,
    /// exactly as the engine spawns it (engine/mod.rs:957).
    reply_tx: mpsc::Sender<String>,
}

impl Interceptor {
    pub fn new(
        iii: Arc<IIIClient>,
        config: Arc<WorkerConfig>,
        catalog: Arc<CatalogCache>,
        session: Arc<ProxySession>,
        reply_tx: mpsc::Sender<String>,
    ) -> Self {
        Self {
            iii,
            config,
            catalog,
            session,
            registered_ids: Mutex::new(HashSet::new()),
            private_ids: Mutex::new(HashSet::new()),
            connection_namespace: Mutex::new(None),
            pending_overrides: Mutex::new(HashMap::new()),
            observers: Mutex::new(ObserverBook::default()),
            inbound: Mutex::new(HashMap::new()),
            warned_removed: Mutex::new(HashSet::new()),
            reply_tx,
        }
    }

    // ----- helpers --------------------------------------------------------

    fn prefix(&self) -> Option<&str> {
        self.session.function_registration_prefix.as_deref()
    }

    /// Apply the session's `{prefix}::` to a bare id (registration path).
    fn apply_prefix(&self, id: &str) -> String {
        match self.prefix() {
            Some(p) => format!("{p}::{id}"),
            None => id.to_string(),
        }
    }

    /// Strip the session's own `{prefix}::` from an id the engine produced
    /// (dispatch / discovery / ack paths). A foreign id is returned unchanged.
    fn strip_prefix(&self, id: &str) -> String {
        match self.prefix() {
            Some(p) => {
                let needle = format!("{p}::");
                id.strip_prefix(&needle).unwrap_or(id).to_string()
            }
            None => id.to_string(),
        }
    }

    /// Strip the session prefix from the string at `key` of an object.
    fn strip_field(&self, v: &mut Value, key: &str) {
        if let Some(id) = v.get(key).and_then(Value::as_str) {
            let stripped = self.strip_prefix(id);
            v[key] = Value::String(stripped);
        }
    }

    /// Whether this connection's own calls of `engine_id`, registered in
    /// `namespace`, may run in-process: always without middleware, which a
    /// local call never reaches, and for a function the connection hosts
    /// privately, whose own calls skip the middleware anyway (the
    /// middleware's forward comes from the control connection, which a
    /// private function refuses).
    async fn local_dispatch(&self, namespace: &str, engine_id: &str) -> bool {
        self.config.middleware_function_id.is_none()
            || (self.is_own(namespace, engine_id).await
                && self.private_ids.lock().await.contains(engine_id))
    }

    /// The namespace this connection's own registrations live in.
    async fn own_namespace(&self) -> String {
        self.connection_namespace
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| DEFAULT_NAMESPACE.to_string())
    }

    /// Resolve a worker-supplied bare `function_id`, reached in `namespace`, to
    /// the id as it exists in the engine registry: a prefixed session's own
    /// registration (`{prefix}::id`) when it registered one in this
    /// namespace, else the canonical (foreign) id. See spec *Prefix
    /// resolution*.
    async fn resolve_target(&self, namespace: &str, function_id: &str) -> String {
        if self.prefix().is_some() && namespace == self.own_namespace().await {
            let candidate = self.apply_prefix(function_id);
            if self.registered_ids.lock().await.contains(&candidate) {
                return candidate;
            }
        }
        function_id.to_string()
    }

    /// Is `engine_id`, reached in `namespace`, a function this session
    /// registered on this connection? The same id in another namespace is
    /// somebody else's.
    async fn is_own(&self, namespace: &str, engine_id: &str) -> bool {
        namespace == self.own_namespace().await
            && self.registered_ids.lock().await.contains(engine_id)
    }

    /// The engine id of a function this connection registered, named by the
    /// bare `function_id` a host frame carries, in the frame's `namespace`
    /// (absent: the connection's own). `None` for any other function.
    async fn own_engine_id(&self, namespace: Option<&Value>, function_id: &str) -> Option<String> {
        let namespace = match namespace {
            None | Some(Value::Null) => self.own_namespace().await,
            Some(Value::String(ns)) => ns.clone(),
            Some(_) => return None,
        };
        let engine_id = self.apply_prefix(function_id);
        self.is_own(&namespace, &engine_id)
            .await
            .then_some(engine_id)
    }

    /// Access-resolution against the live rbac config + this session, for
    /// `engine_id` reached in `namespace`. The target's registered metadata is
    /// fetched from the catalog **only** when the config has a metadata
    /// filter (a cold cache then fails closed for metadata filters);
    /// wildcard-only configs need no catalog round trip.
    async fn allowed(&self, namespace: &str, engine_id: &str) -> bool {
        let metadata = if self.config.rbac.uses_metadata() {
            self.catalog.metadata_for(namespace, engine_id).await
        } else {
            None
        };
        rbac::access_allowed(
            Some(&self.config.rbac),
            &self.session,
            namespace,
            engine_id,
            metadata.as_ref(),
        )
    }

    async fn call_hook(&self, hook_id: &str, input: Value) -> Option<Value> {
        match self.iii.call(CallRequest::new(hook_id, input)).await {
            Ok(v) if v.is_object() => Some(v),
            other => {
                tracing::warn!(hook = %hook_id, result = ?other, "registration hook denied (threw or returned non-object)");
                None
            }
        }
    }

    /// The `caller` to forward for one a worker asserted on a call or a
    /// report: the context it names, written from the call the engine routed
    /// to this connection under that `invocation_id` (the function as the
    /// engine knows it, and that call's own caller as `parent`). `None` when
    /// no such call is in flight, and the field is removed.
    async fn vouched_caller(&self, asserted: Option<&Value>) -> Option<Value> {
        let invocation_id = asserted?.get("invocation_id")?.as_str()?;
        let inbound = self.inbound.lock().await;
        let call = inbound.get(invocation_id)?;
        let mut caller = json!({
            "function_id": call.function_id,
            "invocation_id": invocation_id,
        });
        if let Some(parent) = call.caller.as_ref().and_then(parent_of) {
            caller["parent"] = parent;
        }
        Some(caller)
    }

    async fn warn_removed_frame(&self, frame: &'static str, replacement: &'static str) {
        if self.warned_removed.lock().await.insert(frame) {
            tracing::warn!(
                message_type = %frame,
                "dropping `{frame}` frames from this worker: the iii protocol has no such message; {replacement}"
            );
        }
    }

    // ----- downstream (worker → engine) ----------------------------------

    pub async fn handle_downstream(&self, text: &str) -> DownstreamAction {
        // Not JSON we can classify: dropped, as the engine drops it.
        let Ok(mut v) = serde_json::from_str::<Value>(text) else {
            tracing::debug!("dropping a downstream frame that is not JSON");
            return DownstreamAction::Drop;
        };
        let Some(ty) = v.get("type").and_then(Value::as_str).map(str::to_string) else {
            tracing::debug!("dropping a downstream frame with no type");
            return DownstreamAction::Drop;
        };

        if let Some((frame, replacement)) = REMOVED_FRAMES.iter().copied().find(|(f, _)| *f == ty) {
            self.warn_removed_frame(frame, replacement).await;
            return DownstreamAction::Drop;
        }
        if !DOWNSTREAM_TYPES.contains(&ty.as_str()) {
            tracing::debug!(frame_type = %ty, "dropping a downstream frame type rbac-proxy does not forward");
            return DownstreamAction::Drop;
        }

        match ty.as_str() {
            "registerfunction" => self.on_register_function(v).await,
            "unregisterfunction" => {
                // `id` is a function id; the engine re-prefixes it on
                // unregister via resolve_registration_id, so the proxy must too.
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    let engine_id = self.apply_prefix(id);
                    self.registered_ids.lock().await.remove(&engine_id);
                    self.private_ids.lock().await.remove(&engine_id);
                    let key = (self.own_namespace().await, engine_id.clone());
                    self.observers
                        .lock()
                        .await
                        .hosted
                        .retain(|_, function| *function != key);
                    v["id"] = Value::String(engine_id);
                }
                DownstreamAction::Forward(v.to_string())
            }
            "invokefunction" => self.on_invoke(v).await,
            "invocationresult" => {
                // The answer to a call routed here: its caller stops being
                // one a later frame may name. Correlation is by
                // invocation_id. `matched` and `delivered` pass through: the
                // engine evaluates the observers of the call, among the
                // active observers of the function this connection hosts.
                if let Some(id) = v.get("invocation_id").and_then(Value::as_str) {
                    self.inbound.lock().await.remove(id);
                }
                DownstreamAction::Forward(v.to_string())
            }
            "observe" => self.on_observe(v).await,
            "unobserve" => {
                // The engine disposes only an observer this connection owns,
                // and ignores an id it does not hold (a refused observe's).
                // It still answers every observe sent before this frame; an
                // acceptance among those answers is disposed by this frame.
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    let mut book = self.observers.lock().await;
                    book.created.remove(id);
                    if let Some(queue) = book.requested.get_mut(id) {
                        for requested in queue.iter_mut() {
                            if let Requested::Forwarded { unobserved, .. } = requested {
                                *unobserved = true;
                            }
                        }
                    }
                }
                DownstreamAction::Forward(v.to_string())
            }
            "observerack" => self.on_observer_ack(v).await,
            "invocationsobserved" => self.on_invocations_observed(v).await,
            "localinvocationstats" => self.on_local_invocation_stats(v).await,
            // RegisterService is engine-internal; `Reattach` carries the
            // connection's own token. Both pass through unchanged.
            _ => DownstreamAction::Forward(v.to_string()),
        }
    }

    async fn on_register_function(&self, mut v: Value) -> DownstreamAction {
        if !self.session.allow_function_registration {
            tracing::warn!("function registration not allowed for this session — dropping");
            return DownstreamAction::Drop;
        }

        let bare_id = v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Hook (bare id), then prefix — matches the engine ordering
        // (engine/mod.rs `Message::RegisterFunction`; the code applies the
        // prefix after the hook).
        if let Some(hook_id) = &self.config.rbac.on_function_registration_function_id {
            let namespace = self.own_namespace().await;
            let mut input = json!({
                "function_id": bare_id,
                "description": v.get("description"),
                "metadata": v.get("metadata"),
                "namespace": namespace,
                "private": is_private(&v),
                "observable": is_observable(&v),
                "context": self.session.context,
            });
            if let Some(schema) = v.get("args_format").filter(|s| !s.is_null()) {
                input["args_format"] = schema.clone();
            }
            match self.call_hook(hook_id, input).await {
                Some(mapped) => apply_function_hook_mapping(&mut v, &mapped),
                None => return DownstreamAction::Drop, // deny = silent drop (engine parity)
            }
        }

        let mapped_bare = v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(&bare_id)
            .to_string();
        let engine_id = self.apply_prefix(&mapped_bare);
        self.registered_ids.lock().await.insert(engine_id.clone());
        {
            let mut private_ids = self.private_ids.lock().await;
            if is_private(&v) {
                private_ids.insert(engine_id.clone());
            } else {
                private_ids.remove(&engine_id);
            }
        }
        v["id"] = Value::String(engine_id);
        DownstreamAction::Forward(v.to_string())
    }

    /// `observe`: create an observer of `source` that calls `target`. Gated
    /// like the engine gates a session's observer (the registration hook,
    /// then access to both functions), plus what only the proxy knows: own
    /// registrations resolve to their prefixed ids and are exempt from the
    /// access check, `engine::` targets are refused, and so are sources whose
    /// calls carry what the session's discovery view hides. A refusal is the
    /// engine's answer, `observeresult { state: error }`.
    async fn on_observe(&self, mut v: Value) -> DownstreamAction {
        let Some(id) = v.get("id").and_then(Value::as_str).map(str::to_string) else {
            tracing::debug!("dropping an observe with no string id");
            return DownstreamAction::Drop;
        };
        let (Some(mut source_id), Some(mut target_id)) = (
            v.pointer("/source/function_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            v.pointer("/target/function_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        ) else {
            return self
                .refuse_observe(
                    &id,
                    INVALID_ARGS,
                    "`source.function_id` and `target.function_id` must be strings",
                )
                .await;
        };
        let (Some(source_ns), Some(target_ns)) = (
            requested_namespace(&v["source"]),
            requested_namespace(&v["target"]),
        ) else {
            return self
                .refuse_observe(
                    &id,
                    INVALID_ARGS,
                    "`source.namespace` and `target.namespace` must be strings",
                )
                .await;
        };

        // The observer registration hook (bare ids), with the engine's input;
        // an object answer may rewrite the registration, anything else denies.
        if let Some(hook_id) = &self.config.rbac.on_observer_registration_function_id {
            let args = match v.pointer("/source/args") {
                None | Some(Value::Null) => json!({}),
                Some(args) => args.clone(),
            };
            let input = json!({
                "observer_id": id,
                "function_id": source_id,
                "namespace": source_ns,
                "args": args,
                "filters": non_null(v.pointer("/source/filters")),
                "target_function_id": target_id,
                "target_namespace": target_ns,
                "metadata": non_null(v.get("metadata")),
                "context": self.session.context,
            });
            let Some(answer) = self.call_hook(hook_id, input).await else {
                return self
                    .refuse_observe(
                        &id,
                        "FORBIDDEN",
                        "observer registration denied by the registration hook",
                    )
                    .await;
            };
            if let Some(s) = answer.get("function_id").and_then(Value::as_str) {
                source_id = s.to_string();
            }
            if let Some(s) = answer.get("target_function_id").and_then(Value::as_str) {
                target_id = s.to_string();
            }
            if let Some(args) = answer.get("args") {
                // `null` is no args, which the engine stores as `{}`.
                v["source"]["args"] = if args.is_null() {
                    json!({})
                } else {
                    args.clone()
                };
            }
            if let Some(filters) = answer.get("filters") {
                set_or_remove(&mut v["source"], "filters", filters);
            }
            if let Some(metadata) = answer.get("metadata") {
                set_or_remove(&mut v, "metadata", metadata);
            }
        }

        let source = self.resolve_target(&source_ns, &source_id).await;
        let target = self.resolve_target(&target_ns, &target_id).await;

        // An observer's activations run its target with no session behind
        // them: an engine function would run unrestricted.
        if target.starts_with("engine::") {
            return self
                .refuse_observe(
                    &id,
                    "FORBIDDEN",
                    &format!(
                        "function '{target}' cannot be an observer's target through rbac-proxy"
                    ),
                )
                .await;
        }
        if !observable_through_proxy(&source) {
            return self
                .refuse_observe(
                    &id,
                    "FORBIDDEN",
                    &format!("function '{source}' cannot be observed through rbac-proxy"),
                )
                .await;
        }
        // Observing a function reads every call of it, so it takes what
        // calling it takes, in the namespace each one is reached in.
        for (namespace, engine_id) in [(&source_ns, &source), (&target_ns, &target)] {
            let permitted =
                self.is_own(namespace, engine_id).await || self.allowed(namespace, engine_id).await;
            if !permitted {
                let remediation = rbac::remediation(&self.session, engine_id);
                return self
                    .refuse_observe(
                        &id,
                        "FORBIDDEN",
                        &rbac::forbidden_message(engine_id, remediation),
                    )
                    .await;
            }
        }

        v["source"]["function_id"] = Value::String(source);
        v["target"]["function_id"] = Value::String(target);
        let envelope = matches!(v.pointer("/source/deliver"), None | Some(Value::Null))
            || v.pointer("/source/deliver").and_then(Value::as_str) == Some("envelope");
        self.observers
            .lock()
            .await
            .requested
            .entry(id)
            .or_default()
            .push_back(Requested::Forwarded {
                envelope,
                unobserved: false,
            });
        DownstreamAction::Forward(v.to_string())
    }

    /// The engine's answer to a refused `observe`, `observeresult { state:
    /// error }`: at once, or, while an earlier observe under the id waits for
    /// the engine's answer, right after that answer.
    ///
    /// At once, it follows an `unobserve` of the id to the engine. A
    /// `Reattach` gives this connection the observers of the worker's
    /// previous connection, with no RBAC check: the proxy's connections carry
    /// no session. The worker replays their `observe`s, and a refusal is its
    /// handle's last answer (decision 56), so an observer the session may no
    /// longer have would stay active under the id. The engine ignores an
    /// `unobserve` of an id the connection does not hold, and it reads this
    /// one before anything the worker sends after the refusal.
    async fn refuse_observe(&self, id: &str, code: &str, message: &str) -> DownstreamAction {
        let refusal = observe_refusal(id, code, message);
        let mut book = self.observers.lock().await;
        match book.requested.get_mut(id) {
            Some(queue) => {
                queue.push_back(Requested::Refused(refusal));
                DownstreamAction::Drop
            }
            None => DownstreamAction::ForwardThenReply(unobserve_frame(id), refusal),
        }
    }

    /// `observerack`: a host accepts or rejects an observer of one of its
    /// functions. Only for a function this connection registered.
    async fn on_observer_ack(&self, mut v: Value) -> DownstreamAction {
        let Some(function_id) = v
            .get("function_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return DownstreamAction::Drop;
        };
        let Some(engine_id) = self.own_engine_id(v.get("namespace"), &function_id).await else {
            tracing::debug!(function_id = %function_id, "dropping an observerack for a function this connection does not host");
            return DownstreamAction::Drop;
        };
        v["function_id"] = Value::String(engine_id);
        DownstreamAction::Forward(v.to_string())
    }

    /// `invocationsobserved`: the host's reports of its local calls and the
    /// observers they matched. Each item is kept only for a function this
    /// connection registered, under its engine id, with a `caller` the proxy
    /// vouches for: the context the call was made in. The engine evaluates
    /// the observers of the calls it routes, and drops an item for a call it
    /// routed to the connection.
    async fn on_invocations_observed(&self, mut v: Value) -> DownstreamAction {
        let Some(items) = v
            .get_mut("items")
            .and_then(Value::as_array_mut)
            .map(std::mem::take)
        else {
            return DownstreamAction::Drop;
        };
        let mut kept = Vec::with_capacity(items.len());
        for mut item in items {
            let Some(function_id) = item
                .get("function_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let Some(engine_id) = self
                .own_engine_id(item.get("namespace"), &function_id)
                .await
            else {
                tracing::debug!(function_id = %function_id, "dropping a report item for a function this connection does not host");
                continue;
            };
            item["function_id"] = Value::String(engine_id);
            let caller = self.vouched_caller(item.get("caller")).await;
            set_caller(&mut item, caller);
            kept.push(item);
        }
        if kept.is_empty() {
            return DownstreamAction::Drop;
        }
        v["items"] = Value::Array(kept);
        DownstreamAction::Forward(v.to_string())
    }

    /// `localinvocationstats`: the host's counts of its in-process calls and
    /// report drops. Entries are kept for the functions this connection
    /// registered, under their engine ids.
    async fn on_local_invocation_stats(&self, mut v: Value) -> DownstreamAction {
        let mut any = false;
        for key in ["functions", "dropped_reports"] {
            let Some(entries) = v
                .get_mut(key)
                .and_then(Value::as_array_mut)
                .map(std::mem::take)
            else {
                continue;
            };
            let mut kept = Vec::with_capacity(entries.len());
            for mut entry in entries {
                let Some(function_id) = entry
                    .get("function_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                if let Some(engine_id) = self.own_engine_id(None, &function_id).await {
                    entry["function_id"] = Value::String(engine_id);
                    kept.push(entry);
                }
            }
            any |= !kept.is_empty();
            v[key] = Value::Array(kept);
        }
        if !any {
            return DownstreamAction::Drop;
        }
        DownstreamAction::Forward(v.to_string())
    }

    async fn on_invoke(&self, mut v: Value) -> DownstreamAction {
        let function_id = v
            .get("function_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let inv_id = v
            .get("invocation_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok());

        // A call through an observer (`iii.call({ observer, payload })`).
        let observer_call: Option<Option<String>> = v
            .get("activation")
            .filter(|a| !a.is_null())
            .map(|activation| {
                activation
                    .get("observer_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        match observer_call {
            Some(Some(observer_id)) => {
                return self
                    .on_observer_call(v, &observer_id, &function_id, inv_id)
                    .await
            }
            Some(None) => {
                // Malformed: the engine reads it as absent. It is removed, so
                // the call goes on as an ordinary one without it.
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("activation");
                }
            }
            None => {}
        }

        // The SDK's claim about the call chain, which observers read and
        // filter on: kept only for a call made inside a call routed here.
        // `observe_depth` passes as sent: a lower one only weakens the loop
        // guard for this worker's own calls, as a call from outside any
        // handler does.
        let caller = self.vouched_caller(v.get("caller")).await;
        set_caller(&mut v, caller);

        // The namespace the call is routed in (absent = `default`). A
        // non-string one would fail to decode in the engine, which drops the
        // frame; the proxy drops it too rather than guess.
        let Some(namespace) = requested_namespace(&v) else {
            tracing::debug!(function_id = %function_id, "dropping an invocation whose namespace is not a string");
            return DownstreamAction::Drop;
        };

        // Resolve own-vs-foreign target (engine:: ids are never own; resolve
        // is a no-op for them).
        let target = self.resolve_target(&namespace, &function_id).await;

        // Access resolution (forbidden > scoped grant > allowed > carve-out >
        // expose > deny), in the call's namespace.
        let own = self.is_own(&namespace, &target).await;
        let permitted = own || self.allowed(&namespace, &target).await;
        if !permitted {
            return self.deny_invoke(&function_id, inv_id, &v);
        }

        // An engine:: call reaches the trusted listener as is: queued, it would
        // run later on the queue worker's own connection, as that worker. Only
        // a direct call (no action, or void) goes through.
        if target.starts_with("engine::") && !is_direct_call(&v) {
            return self.reply_invoke_error(
                &function_id,
                inv_id,
                &v,
                "FORBIDDEN",
                &format!("function '{function_id}' may only be called directly through rbac-proxy"),
            );
        }

        if target == "engine::workers::register" {
            return self.on_register_worker(v, inv_id).await;
        }

        if target == DISPOSE_OBSERVER {
            return self.on_dispose_observer(v, &function_id, inv_id).await;
        }

        // Discovery functions: record the override so the result is filtered,
        // then forward (already gated above).
        if engine_overrides::is_discovery(&target) {
            match inv_id {
                Some(id) => {
                    self.pending_overrides
                        .lock()
                        .await
                        .insert(id, target.clone());
                }
                None => tracing::warn!(
                    function_id = %target,
                    "discovery call with no invocation_id — result cannot be filtered (worker discards a void result anyway)"
                ),
            }
            v["function_id"] = Value::String(target);
            return DownstreamAction::Forward(v.to_string());
        }

        // Other engine:: calls (carve-out infra + any exposed engine fn):
        // forward unchanged. Middleware never wraps engine:: (engine parity).
        if target.starts_with("engine::") {
            v["function_id"] = Value::String(target);
            return DownstreamAction::Forward(v.to_string());
        }

        // A private function answers its host only, and the middleware runs
        // on the control connection: this connection's own private functions
        // are called directly.
        if own && self.private_ids.lock().await.contains(&target) {
            v["function_id"] = Value::String(target);
            return DownstreamAction::Forward(v.to_string());
        }

        // Non-engine, allowed: route through middleware if configured. The
        // call is spawned (not awaited inline) so the pump stays free — see
        // `reply_tx`.
        if let Some(mw_id) = self.config.middleware_function_id.clone() {
            self.spawn_middleware(mw_id, target, namespace, &v);
            return DownstreamAction::Drop;
        }

        // No middleware: forward with the resolved target.
        v["function_id"] = Value::String(target);
        DownstreamAction::Forward(v.to_string())
    }

    /// A call through an observer: the engine runs the observer's stored
    /// target with the payload as given, and checks that the sender hosts
    /// the observed function, that the observer is active, and its policy.
    /// The proxy forwards it only for an observer the engine announced to
    /// this connection as the host of its function; otherwise it answers as
    /// the engine would (`NOT_FUNCTION_OWNER`), or drops a fire-and-forget
    /// call, which the engine drops too. No middleware wraps it: it is the
    /// host delivering a call, as an activation is.
    async fn on_observer_call(
        &self,
        mut v: Value,
        observer_id: &str,
        function_id: &str,
        inv_id: Option<Uuid>,
    ) -> DownstreamAction {
        let hosted = self.observers.lock().await.hosted.contains_key(observer_id);
        if !hosted {
            if inv_id.is_none() {
                tracing::debug!(
                    observer_id,
                    "dropping a call through an observer this connection does not host"
                );
                return DownstreamAction::Drop;
            }
            return self.reply_invoke_error(
                function_id,
                inv_id,
                &v,
                "NOT_FUNCTION_OWNER",
                &format!(
                    "observer {observer_id} observes a function this connection does not host"
                ),
            );
        }
        let caller = self.vouched_caller(v.get("caller")).await;
        set_caller(&mut v, caller);
        DownstreamAction::Forward(v.to_string())
    }

    /// `engine::observers::dispose`, already allowed by `expose_functions`:
    /// forwarded only for an observer this connection created. The engine
    /// lets a caller without a session dispose any observer, and the
    /// proxy's connections have none.
    async fn on_dispose_observer(
        &self,
        v: Value,
        function_id: &str,
        inv_id: Option<Uuid>,
    ) -> DownstreamAction {
        let id = v
            .pointer("/data/id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !self.observers.lock().await.created.contains_key(&id) {
            return self.reply_invoke_error(
                function_id,
                inv_id,
                &v,
                "FORBIDDEN",
                &format!("observer `{id}` can only be disposed by the worker that created it"),
            );
        }
        DownstreamAction::Forward(v.to_string())
    }

    /// `engine::workers::register`, already allowed (it is in the carve-out).
    ///
    /// The namespace a connection declares is an authorization decision: once
    /// the auth function scopes the session to a set of namespaces, declaring
    /// one outside them is refused exactly as the engine refuses it — the
    /// call's `FORBIDDEN` result, a `RegistrationRejected { code:
    /// NAMESPACE_FORBIDDEN }`, then Close, so the worker cannot keep running
    /// (and registering) in `default`. An unscoped session declares freely.
    ///
    /// The worker's `features` are reduced to the ones the proxy enforces
    /// ([`SUPPORTED_FEATURES`]).
    async fn on_register_worker(&self, mut v: Value, inv_id: Option<Uuid>) -> DownstreamAction {
        let declared = v
            .get("data")
            .and_then(|d| d.get("namespace"))
            .and_then(Value::as_str)
            .map(str::to_string);

        if !self.session.namespaces.is_empty() {
            let checked = declared.as_deref().unwrap_or(DEFAULT_NAMESPACE);
            if !self.session.namespaces.contains_key(checked) {
                let mut permitted: Vec<&str> =
                    self.session.namespaces.keys().map(String::as_str).collect();
                permitted.sort_unstable();
                let mut frames = Vec::new();
                if let Some(id) = inv_id {
                    frames.push(
                        json!({
                            "type": "invocationresult",
                            "invocation_id": id.to_string(),
                            "function_id": "engine::workers::register",
                            "error": {
                                "code": "FORBIDDEN",
                                "message": format!(
                                    "namespace '{checked}' not allowed for this session (permitted: {})",
                                    permitted.join(", ")
                                ),
                            },
                        })
                        .to_string(),
                    );
                }
                let mut rejected = json!({
                    "type": "registrationrejected",
                    "code": NAMESPACE_FORBIDDEN,
                    "namespace": checked,
                    "owner_worker_id": "",
                });
                if let Some(name) = v
                    .get("data")
                    .and_then(|d| d.get("name"))
                    .and_then(Value::as_str)
                {
                    rejected["worker_name"] = Value::String(name.to_string());
                }
                frames.push(rejected.to_string());
                tracing::warn!(namespace = %checked, "namespace not allowed for this session — closing the connection");
                return DownstreamAction::Close(frames);
            }
        }

        // The engine keeps the last namespace a worker declared; a declaration
        // without one leaves it unchanged.
        if let Some(namespace) = declared {
            *self.connection_namespace.lock().await = Some(namespace);
        }
        if let Some(data) = v.get_mut("data").and_then(Value::as_object_mut) {
            retain_supported_features(data);
            // The name carries `rbac-proxy:` and the session prefix: a proxied
            // worker never holds a trusted worker's name, and another
            // prefix's session declaring the same name is another worker, to
            // the engine's one-live-worker-per-name rule and to the
            // replacement of a suspended observer (decision 51).
            if let Some(Value::String(name)) = data.get_mut("name") {
                *name = self.session.proxied_worker_name(name);
            }
        }
        DownstreamAction::Forward(v.to_string())
    }

    /// Synthesize the engine's `FORBIDDEN` `InvocationResult` (engine/mod.rs:
    /// 911-937). The engine always replies — even for a `void` action, where
    /// it fabricates an `invocation_id`; the proxy mirrors that.
    fn deny_invoke(&self, function_id: &str, inv_id: Option<Uuid>, v: &Value) -> DownstreamAction {
        let remediation = rbac::remediation(&self.session, function_id);
        self.reply_invoke_error(
            function_id,
            inv_id,
            v,
            "FORBIDDEN",
            &rbac::forbidden_message(function_id, remediation),
        )
    }

    /// An `InvocationResult { error: { code, message } }` for the caller, with
    /// the engine's conventions: always sent, even for a `void` call (with a
    /// fabricated `invocation_id`), and echoing the trace context.
    fn reply_invoke_error(
        &self,
        function_id: &str,
        inv_id: Option<Uuid>,
        v: &Value,
        code: &str,
        message: &str,
    ) -> DownstreamAction {
        let id = inv_id.unwrap_or_else(Uuid::new_v4);
        let mut result = json!({
            "type": "invocationresult",
            "invocation_id": id.to_string(),
            "function_id": function_id,
            "error": { "code": code, "message": message },
        });
        // Echo trace context, as the engine does.
        if let Some(tp) = v.get("traceparent") {
            result["traceparent"] = tp.clone();
        }
        if let Some(bg) = v.get("baggage") {
            result["baggage"] = bg.clone();
        }
        DownstreamAction::ReplyToClient(result.to_string())
    }

    /// Invoke the middleware on the control connection in a **spawned task**;
    /// its return value becomes the `InvocationResult` for the caller
    /// (engine/mod.rs:940-980), delivered out of band via `reply_tx`. `engine::*`
    /// calls never reach here. Fail closed: an unreachable / erroring middleware
    /// denies the call, it never silently forwards.
    ///
    /// Spawning (rather than awaiting inline) is load-bearing: the middleware
    /// may invoke a function owned by *this* connection (e.g. a prefixed
    /// session calling its own handler), whose dispatched result must flow back
    /// through the downstream→engine pump. Awaiting inline parks that pump and
    /// deadlocks the round trip.
    ///
    /// As in the engine, the middleware is resolved in the caller's namespace
    /// and told that namespace (so it can re-target the same one), and it
    /// receives the call's metadata sidecar.
    fn spawn_middleware(&self, mw_id: String, target: String, namespace: String, v: &Value) {
        let inv_id = v
            .get("invocation_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .unwrap_or_else(Uuid::new_v4);
        let input = json!({
            "function_id": target,
            "payload": v.get("data").cloned().unwrap_or(Value::Null),
            "action": v.get("action").cloned().unwrap_or(Value::Null),
            "context": self.session.context,
            "namespace": namespace,
        });
        let metadata = v.get("metadata").filter(|m| !m.is_null()).cloned();
        let traceparent = v.get("traceparent").cloned();
        let baggage = v.get("baggage").cloned();
        let iii = self.iii.clone();
        let reply_tx = self.reply_tx.clone();

        tokio::spawn(async move {
            let mut result = json!({
                "type": "invocationresult",
                "invocation_id": inv_id.to_string(),
                "function_id": target,
            });
            let mut request = CallRequest {
                function_id: mw_id,
                payload: input,
                action: None,
                timeout_ms: Some(MIDDLEWARE_TIMEOUT_MS),
            }
            .namespace(namespace);
            if let Some(metadata) = metadata {
                request = request.metadata(metadata);
            }
            match iii.call(request).await {
                Ok(value) => {
                    result["result"] = value;
                }
                Err(e) => {
                    let (code, message) = error_body(&e);
                    result["error"] = json!({ "code": code, "message": message });
                }
            }
            if let Some(tp) = traceparent {
                result["traceparent"] = tp;
            }
            if let Some(bg) = baggage {
                result["baggage"] = bg;
            }
            let _ = reply_tx.send(result.to_string()).await;
        });
    }

    // ----- upstream (engine → worker) ------------------------------------

    pub async fn handle_upstream(&self, text: &str) -> UpstreamAction {
        let Ok(mut v) = serde_json::from_str::<Value>(text) else {
            tracing::debug!("dropping an upstream frame that is not JSON");
            return UpstreamAction::Drop;
        };
        let Some(ty) = v.get("type").and_then(Value::as_str).map(str::to_string) else {
            tracing::debug!("dropping an upstream frame with no type");
            return UpstreamAction::Drop;
        };
        if !UPSTREAM_TYPES.contains(&ty.as_str()) {
            tracing::debug!(frame_type = %ty, "dropping an upstream frame type rbac-proxy does not forward");
            return UpstreamAction::Drop;
        }

        match ty.as_str() {
            // The engine's protocol features, reduced to what the proxy
            // enforces, so the SDK speaks the protocol the proxy can gate.
            "workerregistered" => {
                if let Some(obj) = v.as_object_mut() {
                    retain_supported_features(obj);
                }
                UpstreamAction::Forward(v.to_string())
            }
            "registrationrejected" => {
                // A registration the engine refused (a standby's
                // FUNCTION_NAMESPACE_CONFLICT included) is not this
                // connection's: its id stops being own. Strip the prefix so
                // the worker sees the id it sent.
                if let Some(fid) = v
                    .get("function_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    self.registered_ids.lock().await.remove(&fid);
                    self.private_ids.lock().await.remove(&fid);
                    v["function_id"] = Value::String(self.strip_prefix(&fid));
                }
                UpstreamAction::Forward(v.to_string())
            }
            "invokefunction" => self.on_dispatch(v).await,
            // Result of a call the worker made: rewrite if it is a recorded
            // discovery override; else pass through.
            "invocationresult" => self.on_invocation_result(v).await,
            "functionregistered" => self.on_function_registered(v).await,
            "observeradded" => {
                if let (Some(function_id), Some(observer_id)) = (
                    v.get("function_id").and_then(Value::as_str),
                    v.pointer("/observer/id").and_then(Value::as_str),
                ) {
                    let key = (frame_namespace(&v), function_id.to_string());
                    self.observers
                        .lock()
                        .await
                        .hosted
                        .insert(observer_id.to_string(), key);
                }
                self.strip_field(&mut v, "function_id");
                if let Some(observer) = v.get_mut("observer") {
                    self.strip_field(observer, "target");
                }
                UpstreamAction::Forward(v.to_string())
            }
            "observerremoved" => {
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    self.observers.lock().await.hosted.remove(id);
                }
                self.strip_field(&mut v, "function_id");
                UpstreamAction::Forward(v.to_string())
            }
            // The engine's answer to this connection's oldest unanswered
            // `observe` under the id. It ends that entry: a refusal is that
            // observe's final answer.
            //
            // A refusal can leave an observer under the id: the replay of an
            // observer a `Reattach` gave this connection, refused because the
            // registration hook rewrote it otherwise for this session
            // (`OBSERVER_ID_CONFLICT`, a different spec), stays active as the
            // earlier session had it. So an `unobserve` of the id goes to the
            // engine first, unless the worker sent one after the observe or
            // another observe of the id waits on its answer.
            "observeresult" => {
                let mut refusals = Vec::new();
                let mut unobserve = None;
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    let state = v.get("state").and_then(Value::as_str);
                    let ended = matches!(state, Some("error" | "disposed"));
                    let mut book = self.observers.lock().await;
                    let mut answered = None;
                    let mut waiting = false;
                    if let Some(queue) = book.requested.get_mut(id) {
                        answered = queue.pop_front();
                        while let Some(Requested::Refused(_)) = queue.front() {
                            if let Some(Requested::Refused(refusal)) = queue.pop_front() {
                                refusals.push(refusal);
                            }
                        }
                        waiting = !queue.is_empty();
                        if !waiting {
                            book.requested.remove(id);
                        }
                    }
                    if let Some(Requested::Forwarded {
                        envelope,
                        unobserved: false,
                    }) = answered
                    {
                        if !ended {
                            book.created.insert(id.to_string(), envelope);
                        } else if state == Some("error") && !waiting {
                            unobserve = Some(unobserve_frame(id));
                        }
                    }
                    if state == Some("disposed") {
                        book.created.remove(id);
                    }
                }
                refusals.insert(0, v.to_string());
                match unobserve {
                    Some(unobserve) => UpstreamAction::ReplyThenForwardAll(unobserve, refusals),
                    None if refusals.len() == 1 => UpstreamAction::Forward(refusals.remove(0)),
                    None => UpstreamAction::ForwardAll(refusals),
                }
            }
            "observerstatechanged" => {
                if v.get("state").and_then(Value::as_str) == Some("disposed") {
                    if let Some(id) = v.get("id").and_then(Value::as_str) {
                        self.observers.lock().await.created.remove(id);
                    }
                }
                UpstreamAction::Forward(v.to_string())
            }
            _ => UpstreamAction::Forward(v.to_string()),
        }
    }

    /// The engine dispatching a call TO the worker: an ordinary call, an
    /// activation of an observer whose target the worker hosts, or a call
    /// through an observer. Strip the session's own prefix so the worker SDK
    /// finds its local (bare) handler (worker_connections/traits.rs:105-117),
    /// and remember the call until the worker answers it, so a `caller` the
    /// worker asserts can be checked against it.
    async fn on_dispatch(&self, mut v: Value) -> UpstreamAction {
        let engine_id = v
            .get("function_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let (Some(invocation_id), Some(function_id)) = (
            v.get("invocation_id").and_then(Value::as_str),
            engine_id.as_ref(),
        ) {
            self.inbound.lock().await.insert(
                invocation_id.to_string(),
                InboundCall {
                    function_id: function_id.clone(),
                    caller: v.get("caller").filter(|c| c.is_object()).cloned(),
                },
            );
        }

        // iii's delivery to an observer this connection created: its
        // `Observed` envelope names the observed function by its engine id.
        // A call through the same observer carries the host's payload, which
        // goes to the target as the host sent it.
        if let Some(observer_id) = v.pointer("/activation/observer_id").and_then(Value::as_str) {
            let envelope = self
                .observers
                .lock()
                .await
                .created
                .get(observer_id)
                .copied()
                .unwrap_or(false);
            if envelope && carries_its_envelope(&v) {
                if let Some(data) = v.get_mut("data") {
                    self.strip_field(data, "function_id");
                }
            }
        }

        if let Some(engine_id) = engine_id {
            v["function_id"] = Value::String(self.strip_prefix(&engine_id));
        }
        UpstreamAction::Forward(v.to_string())
    }

    /// `functionregistered`: the engine's ack of one of this connection's
    /// registrations, with every observer of the function. The observers are
    /// the ones this connection may call through; ids reach the worker
    /// without the session prefix. While middleware is configured,
    /// `local_dispatch` stays `true` only for a function the connection hosts
    /// privately (decision 46): its own calls of any other function run on
    /// the wire, through the middleware.
    async fn on_function_registered(&self, mut v: Value) -> UpstreamAction {
        let mut local_dispatch = false;
        if let Some(engine_id) = v.get("id").and_then(Value::as_str) {
            let key = (frame_namespace(&v), engine_id.to_string());
            local_dispatch = self.local_dispatch(&key.0, engine_id).await;
            let ids: Vec<String> = v
                .get("observers")
                .and_then(Value::as_array)
                .map(|observers| {
                    observers
                        .iter()
                        .filter_map(|o| o.get("id").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let mut book = self.observers.lock().await;
            book.hosted.retain(|_, function| *function != key);
            for id in ids {
                book.hosted.insert(id, key.clone());
            }
        }
        self.strip_field(&mut v, "id");
        if let Some(observers) = v.get_mut("observers").and_then(Value::as_array_mut) {
            for observer in observers {
                self.strip_field(observer, "target");
            }
        }
        if !local_dispatch {
            v["local_dispatch"] = Value::Bool(false);
        }
        UpstreamAction::Forward(v.to_string())
    }

    async fn on_invocation_result(&self, mut v: Value) -> UpstreamAction {
        let inv_id = v
            .get("invocation_id")
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok());

        let discovery_fn = match inv_id {
            Some(id) => self.pending_overrides.lock().await.remove(&id),
            None => None,
        };

        let Some(function_id) = discovery_fn else {
            return UpstreamAction::Forward(v.to_string());
        };

        // The engine may itself have returned an error (e.g. NOT_FOUND);
        // nothing to filter — pass it through.
        let Some(result) = v.get("result").cloned() else {
            return UpstreamAction::Forward(v.to_string());
        };
        if result.is_null() {
            return UpstreamAction::Forward(v.to_string());
        }

        let own_observers: HashSet<String> = self
            .observers
            .lock()
            .await
            .created
            .keys()
            .cloned()
            .collect();
        match engine_overrides::filter_result(
            &function_id,
            result,
            &self.session,
            &self.config,
            &self.catalog,
            &own_observers,
        )
        .await
        {
            OverrideOutcome::Result(filtered) => {
                v["result"] = filtered;
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("error");
                }
            }
            OverrideOutcome::Error { code, message } => {
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("result");
                    obj.insert(
                        "error".to_string(),
                        json!({ "code": code, "message": message }),
                    );
                }
            }
        }
        UpstreamAction::Forward(v.to_string())
    }
}

/// `unobserve` of `id`, as a worker sends it.
fn unobserve_frame(id: &str) -> String {
    json!({ "type": "unobserve", "id": id }).to_string()
}

/// The engine's answer to an `observe` it refuses: `observeresult { state:
/// "error", error: { code, message } }`.
fn observe_refusal(id: &str, code: &str, message: &str) -> String {
    tracing::debug!(observer_id = %id, code, message, "refusing observe");
    json!({
        "type": "observeresult",
        "id": id,
        "state": "error",
        "error": { "code": code, "message": message },
    })
    .to_string()
}

/// Whether `engine_id` may be an observer's source through the proxy. The
/// infrastructure carve-out is for calling: the calls of those functions
/// carry other workers' channel keys, logs and registrations. The discovery
/// functions, the observer builtins and the engine events that list
/// functions, workers or observers carry what the discovery overrides filter.
fn observable_through_proxy(engine_id: &str) -> bool {
    !(rbac::INFRASTRUCTURE_FUNCTIONS.contains(&engine_id)
        || engine_overrides::is_discovery(engine_id)
        || engine_id.starts_with(OBSERVER_FUNCTIONS_PREFIX)
        || UNFILTERED_SOURCES.contains(&engine_id))
}

/// The namespace a frame names in `namespace`: absent or `null` is
/// `default`; any other non-string is `None` (the engine would fail to decode
/// the frame).
fn requested_namespace(v: &Value) -> Option<String> {
    match v.get("namespace") {
        None | Some(Value::Null) => Some(DEFAULT_NAMESPACE.to_string()),
        Some(Value::String(ns)) => Some(ns.clone()),
        Some(_) => None,
    }
}

/// Whether an activation's `data` is the engine's `Observed` envelope for
/// that activation: the envelope names the activation's `observer_id` and
/// `activation_id`. A call through an observer gets a fresh `activation_id`
/// from the engine, so a host's payload never names it.
fn carries_its_envelope(v: &Value) -> bool {
    let (Some(activation), Some(data)) = (v.get("activation"), v.get("data")) else {
        return false;
    };
    ["observer_id", "activation_id"].into_iter().all(|key| {
        matches!(
            (activation.get(key), data.get(key)),
            (Some(Value::String(a)), Some(Value::String(d))) if a == d
        )
    })
}

/// The `namespace` of an engine frame about a function (`default` when it
/// names none).
fn frame_namespace(v: &Value) -> String {
    v.get("namespace")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_NAMESPACE)
        .to_string()
}

/// `value` unless it is `null` or absent.
fn non_null(value: Option<&Value>) -> Value {
    value
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Null)
}

/// Set `key` of the object `v` to `value`, or remove it for `null`.
fn set_or_remove(v: &mut Value, key: &str, value: &Value) {
    if let Some(obj) = v.as_object_mut() {
        if value.is_null() {
            obj.remove(key);
        } else {
            obj.insert(key.to_string(), value.clone());
        }
    }
}

/// Set a frame's `caller`, or remove it.
fn set_caller(v: &mut Value, caller: Option<Value>) {
    if let Some(obj) = v.as_object_mut() {
        match caller {
            Some(caller) => {
                obj.insert("caller".to_string(), caller);
            }
            None => {
                obj.remove("caller");
            }
        }
    }
}

/// A caller as the `parent` of another: one level, its own `parent` left
/// out. `None` when it names nothing.
fn parent_of(caller: &Value) -> Option<Value> {
    let mut parent = serde_json::Map::new();
    for key in ["worker_id", "worker_name", "function_id", "invocation_id"] {
        if let Some(value) = caller.get(key).filter(|v| v.is_string()) {
            parent.insert(key.to_string(), value.clone());
        }
    }
    (!parent.is_empty()).then_some(Value::Object(parent))
}

/// Keep only the features the proxy enforces in an object's `features` list
/// ([`SUPPORTED_FEATURES`]), and drop the key when none is left (an absent
/// list reads as "no features" on both sides).
fn retain_supported_features(obj: &mut serde_json::Map<String, Value>) {
    let kept: Vec<Value> = obj
        .get("features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .filter(|f| f.as_str().is_some_and(|f| SUPPORTED_FEATURES.contains(&f)))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if kept.is_empty() {
        obj.remove("features");
    } else {
        obj.insert("features".to_string(), Value::Array(kept));
    }
}

/// No `action`, or a void one: the call runs now, on the caller's connection.
fn is_direct_call(v: &Value) -> bool {
    match v.get("action") {
        None | Some(Value::Null) => true,
        Some(action) => action.get("type").and_then(Value::as_str) == Some("void"),
    }
}

/// A registration's `private` (absent: `false`).
fn is_private(v: &Value) -> bool {
    v.get("private").and_then(Value::as_bool).unwrap_or(false)
}

/// A registration's `observable` (absent: `true`), with a 0.25 SDK's
/// `observe: { allow: false }` read as `false`, as the engine reads it.
fn is_observable(v: &Value) -> bool {
    v.get("observable").and_then(Value::as_bool).unwrap_or(true)
        && v.pointer("/observe/allow") != Some(&Value::Bool(false))
}

/// Set a registration flag, leaving it out at its default as SDKs do.
fn set_flag(v: &mut Value, key: &str, value: bool, default: bool) {
    if let Some(obj) = v.as_object_mut() {
        if value == default {
            obj.remove(key);
        } else {
            obj.insert(key.to_string(), Value::Bool(value));
        }
    }
}

/// Map iii-sdk `Error` to a `{code, message}` for a synthesized error result.
/// A remote function error preserves its code; transport/availability errors
/// surface as a clearly fail-closed code so a broken control plane denies.
fn error_body(e: &Error) -> (String, String) {
    match e {
        Error::Remote { code, message, .. } => (code.clone(), message.clone()),
        Error::Timeout => ("TIMEOUT".to_string(), e.to_string()),
        Error::NotConnected => (
            "MIDDLEWARE_UNAVAILABLE".to_string(),
            "middleware/control connection unavailable".to_string(),
        ),
        _ => ("MIDDLEWARE_ERROR".to_string(), e.to_string()),
    }
}

/// Apply an `on_function_registration` hook's optional `{ function_id?,
/// description?, metadata?, private?, observable?, args_format? }` mapping
/// onto the frame (omitted ⇒ unchanged). Each of `private`, `observable` and
/// `args_format` present replaces the registration's value, the 0.25
/// `observe.allow` included.
fn apply_function_hook_mapping(v: &mut Value, mapped: &Value) {
    if let Some(s) = mapped.get("function_id").and_then(Value::as_str) {
        v["id"] = Value::String(s.to_string());
    }
    if let Some(d) = mapped.get("description") {
        v["description"] = d.clone();
    }
    if let Some(m) = mapped.get("metadata") {
        v["metadata"] = m.clone();
    }
    if let Some(private) = mapped.get("private").and_then(Value::as_bool) {
        set_flag(v, "private", private, false);
    }
    if let Some(observable) = mapped.get("observable").and_then(Value::as_bool) {
        set_flag(v, "observable", observable, true);
        if let Some(descriptor) = v.get_mut("observe").and_then(Value::as_object_mut) {
            descriptor.remove("allow");
        }
    }
    if let Some(schema) = mapped.get("args_format") {
        set_or_remove(v, "args_format", schema);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_overrides::CatalogCache;
    use crate::rbac::{FunctionFilter, RbacConfig};

    fn iii() -> Arc<IIIClient> {
        // A disconnected client is enough for the engine-free interception
        // tests: every path exercised here decides locally and never reaches
        // the control connection (no auth/middleware/hook/discovery calls).
        Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:1",
            iii_sdk::InitOptions::default(),
        ))
    }

    fn config_with(rbac: RbacConfig, middleware: Option<&str>) -> Arc<WorkerConfig> {
        Arc::new(WorkerConfig {
            middleware_function_id: middleware.map(str::to_string),
            rbac,
            ..WorkerConfig::default()
        })
    }

    fn expose(patterns: &[&str]) -> RbacConfig {
        RbacConfig {
            expose_functions: patterns
                .iter()
                .map(|p| FunctionFilter::match_pattern(p))
                .collect(),
            ..RbacConfig::default()
        }
    }

    fn session(prefix: Option<&str>) -> Arc<ProxySession> {
        Arc::new(ProxySession {
            function_registration_prefix: prefix.map(str::to_string),
            ..ProxySession::permissive("1.2.3.4".to_string())
        })
    }

    fn interceptor(cfg: Arc<WorkerConfig>, sess: Arc<ProxySession>) -> Interceptor {
        // Keep the receiver alive for the test process so any `reply_tx` send
        // (the middleware path) doesn't error; these tests don't assert on it.
        let (tx, rx) = mpsc::channel::<String>(16);
        Box::leak(Box::new(rx));
        Interceptor::new(iii(), cfg, Arc::new(CatalogCache::new(iii())), sess, tx)
    }

    /// The synthesized reply; a refused observe's goes after an `unobserve`
    /// of its id to the engine.
    fn reply(action: DownstreamAction) -> Value {
        match action {
            DownstreamAction::ReplyToClient(text) => serde_json::from_str(&text).unwrap(),
            DownstreamAction::ForwardThenReply(out, text) => {
                let reply: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(reply["type"], "observeresult");
                assert_eq!(
                    serde_json::from_str::<Value>(&out).unwrap(),
                    json!({ "type": "unobserve", "id": reply["id"] })
                );
                reply
            }
            other => panic!("expected a synthesized reply, got {other:?}"),
        }
    }

    fn forwarded(action: DownstreamAction) -> Value {
        let DownstreamAction::Forward(text) = action else {
            panic!("expected a forward, got {action:?}");
        };
        serde_json::from_str(&text).unwrap()
    }

    fn forwarded_up(action: UpstreamAction) -> Value {
        let UpstreamAction::Forward(text) = action else {
            panic!("expected a forward, got {action:?}");
        };
        serde_json::from_str(&text).unwrap()
    }

    async fn register(i: &Interceptor, id: &str) {
        let frame = json!({ "type": "registerfunction", "id": id });
        forwarded(i.handle_downstream(&frame.to_string()).await);
    }

    #[tokio::test]
    async fn denied_invoke_synthesizes_forbidden_with_echoed_id() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "secret::do",
            "data": {}
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["type"], "invocationresult");
        assert_eq!(r["invocation_id"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(r["function_id"], "secret::do");
        assert_eq!(r["error"]["code"], "FORBIDDEN");
        assert_eq!(
            r["error"]["message"],
            "function 'secret::do' not allowed (add to rbac.expose_functions)"
        );
    }

    #[tokio::test]
    async fn queued_engine_call_is_refused() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "22222222-2222-2222-2222-222222222222",
            "function_id": "engine::workers::register",
            "data": { "name": "browser:x", "runtime": "browser" },
            "action": { "type": "enqueue", "queue": "default" }
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    /// Two sessions without a prefix declaring one name are two workers to
    /// the engine: neither shuts the other out, nor takes its suspended
    /// observers (decision 51).
    #[tokio::test]
    async fn declared_worker_name_is_prefixed_and_unprefixed_sessions_are_apart() {
        let frame = json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": { "name": "queue", "runtime": "browser" },
            "action": { "type": "void" }
        })
        .to_string();
        let mut names = Vec::new();
        for _ in 0..2 {
            let sess = session(None);
            let i = interceptor(config_with(expose(&["api::*"]), None), sess.clone());
            let v = forwarded(i.handle_downstream(&frame).await);
            assert_eq!(
                v["data"]["name"],
                format!("rbac-proxy:{}:queue", sess.session_id)
            );
            names.push(v["data"]["name"].clone());
        }
        assert_ne!(names[0], names[1]);
    }

    /// Decision 51 replaces a suspended observer from a connection under the
    /// same worker name and namespace, so sessions under different prefixes
    /// declaring one name must reach the engine as different workers.
    #[tokio::test]
    async fn declared_worker_name_carries_the_session_prefix() {
        let frame = json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": { "name": "queue", "runtime": "node" },
            "action": { "type": "void" }
        })
        .to_string();
        let mut names = Vec::new();
        for prefix in ["tenant1", "tenant2"] {
            let i = interceptor(
                config_with(expose(&["api::*"]), None),
                session(Some(prefix)),
            );
            let v = forwarded(i.handle_downstream(&frame).await);
            names.push(v["data"]["name"].clone());
        }
        assert_eq!(
            names,
            vec![
                json!("rbac-proxy:tenant1::queue"),
                json!("rbac-proxy:tenant2::queue")
            ]
        );
    }

    #[tokio::test]
    async fn void_worker_registration_still_goes_through() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": { "name": "browser:x", "runtime": "browser" },
            "action": { "type": "void" }
        });
        forwarded(i.handle_downstream(&frame.to_string()).await);
    }

    #[tokio::test]
    async fn denied_void_invoke_fabricates_invocation_id() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": null,
            "function_id": "secret::do",
            "data": {},
            "action": { "type": "void" }
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        // A real (non-null) UUID was fabricated.
        assert!(Uuid::parse_str(r["invocation_id"].as_str().unwrap()).is_ok());
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn forbidden_remediation_for_explicitly_forbidden() {
        let mut s = ProxySession::permissive("ip".to_string());
        s.forbidden_functions = vec!["api::danger".to_string()];
        let i = interceptor(config_with(expose(&["api::*"]), None), Arc::new(s));
        let frame = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"api::danger","data":{}});
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(
            r["error"]["message"],
            "function 'api::danger' not allowed (remove from rbac.forbidden_functions)"
        );
    }

    #[tokio::test]
    async fn allowed_invoke_forwards_unchanged_without_prefix() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"api::users::list","data":{"q":1}});
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(o["function_id"], "api::users::list");
        assert_eq!(o["data"]["q"], 1);
    }

    #[tokio::test]
    async fn register_function_applies_prefix_and_tracks_own_id() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"registerfunction","id":"foo","request_format":null,"response_format":null});
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(o["id"], "tenant1::foo");
        assert!(i.registered_ids.lock().await.contains("tenant1::foo"));
    }

    #[tokio::test]
    async fn register_function_forwards_the_v2_options() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({
            "type": "registerfunction",
            "id": "cron::on-tick",
            "args_format": { "type": "object", "required": ["expression"] },
            "private": true
        });
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(o["id"], "tenant1::cron::on-tick");
        assert_eq!(o["private"], true);
        assert_eq!(o["args_format"], frame["args_format"]);
        assert!(i
            .private_ids
            .lock()
            .await
            .contains("tenant1::cron::on-tick"));

        // Registered again without `private`: no longer one of them.
        register(&i, "cron::on-tick").await;
        assert!(!i
            .private_ids
            .lock()
            .await
            .contains("tenant1::cron::on-tick"));

        let not_observable =
            json!({ "type": "registerfunction", "id": "vault::read", "observable": false });
        let o = forwarded(i.handle_downstream(&not_observable.to_string()).await);
        assert_eq!(o["observable"], false);
    }

    #[test]
    fn a_registration_hook_answer_replaces_the_v2_options() {
        let mut frame = json!({
            "type": "registerfunction",
            "id": "w::source",
            "args_format": { "type": "object" },
            "observe": { "delivery": "auto", "allow": false }
        });
        assert!(!is_observable(&frame));
        apply_function_hook_mapping(
            &mut frame,
            &json!({ "private": true, "observable": true, "args_format": null }),
        );
        assert_eq!(frame["private"], true);
        assert!(frame.get("observable").is_none(), "true is the default");
        assert_eq!(frame["observe"], json!({ "delivery": "auto" }));
        assert!(frame.get("args_format").is_none());
        assert!(is_private(&frame) && is_observable(&frame));

        apply_function_hook_mapping(
            &mut frame,
            &json!({ "private": false, "observable": false }),
        );
        assert!(frame.get("private").is_none());
        assert_eq!(frame["observable"], false);
    }

    #[tokio::test]
    async fn prefixed_session_self_invokes_own_bare_function() {
        // Register `foo` (stored tenant1::foo), then invoke bare `foo`.
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "foo").await;

        let inv = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"foo","data":{}});
        let o = forwarded(i.handle_downstream(&inv.to_string()).await);
        // Resolved to the engine id.
        assert_eq!(o["function_id"], "tenant1::foo");
    }

    #[tokio::test]
    async fn own_private_functions_skip_the_middleware() {
        let i = interceptor(
            config_with(expose(&[]), Some("p::mw")),
            session(Some("tenant1")),
        );
        let private = json!({ "type": "registerfunction", "id": "source", "private": true });
        forwarded(i.handle_downstream(&private.to_string()).await);
        register(&i, "plain").await;

        let call = |id: &str| {
            json!({
                "type": "invokefunction",
                "invocation_id": "11111111-1111-1111-1111-111111111111",
                "function_id": id,
                "data": {}
            })
            .to_string()
        };
        let o = forwarded(i.handle_downstream(&call("source")).await);
        assert_eq!(o["function_id"], "tenant1::source");
        // An ordinary own function goes through the middleware (spawned).
        assert_eq!(
            i.handle_downstream(&call("plain")).await,
            DownstreamAction::Drop
        );
    }

    #[tokio::test]
    async fn foreign_invoke_matches_expose_unprefixed() {
        let i = interceptor(
            config_with(expose(&["api::*"]), None),
            session(Some("tenant1")),
        );
        let inv = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"api::users::list","data":{}});
        let o = forwarded(i.handle_downstream(&inv.to_string()).await);
        assert_eq!(o["function_id"], "api::users::list");
    }

    #[tokio::test]
    async fn register_function_denied_when_disallowed() {
        let mut s = ProxySession::permissive("ip".to_string());
        s.allow_function_registration = false;
        let i = interceptor(config_with(expose(&[]), None), Arc::new(s));
        let frame = json!({"type":"registerfunction","id":"foo","request_format":null,"response_format":null});
        assert_eq!(
            i.handle_downstream(&frame.to_string()).await,
            DownstreamAction::Drop
        );
    }

    #[tokio::test]
    async fn a_rejected_registration_stops_being_own() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "foo").await;
        let rejected = json!({
            "type": "registrationrejected",
            "code": "FUNCTION_NAMESPACE_CONFLICT",
            "namespace": "default",
            "function_id": "tenant1::foo",
            "owner_worker_id": "7c1e0b52-93d4-4f0e-8a3b-6d2f1c9e4a70"
        });
        let o = forwarded_up(i.handle_upstream(&rejected.to_string()).await);
        assert_eq!(o["function_id"], "foo");

        let inv = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"tenant1::foo","data":{}});
        let r = reply(i.handle_downstream(&inv.to_string()).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn upstream_dispatch_strips_prefix() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"invokefunction","invocation_id":"22222222-2222-2222-2222-222222222222","function_id":"tenant1::foo","data":{}});
        let o = forwarded_up(i.handle_upstream(&frame.to_string()).await);
        assert_eq!(o["function_id"], "foo");
    }

    #[tokio::test]
    async fn unknown_frame_types_are_dropped_both_ways() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let frame = json!({"type":"ping"});
        assert_eq!(
            i.handle_downstream(&frame.to_string()).await,
            DownstreamAction::Forward(frame.to_string())
        );
        let future = json!({"type":"somefutureframe","x":1});
        assert_eq!(
            i.handle_downstream(&future.to_string()).await,
            DownstreamAction::Drop
        );
        assert_eq!(
            i.handle_upstream(&future.to_string()).await,
            UpstreamAction::Drop
        );
        for text in ["not json", "{\"no_type\":1}", "[1,2]"] {
            assert_eq!(i.handle_downstream(text).await, DownstreamAction::Drop);
            assert_eq!(i.handle_upstream(text).await, UpstreamAction::Drop);
        }
    }

    #[tokio::test]
    async fn removed_frames_are_dropped_with_one_warning_each() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        for _ in 0..2 {
            for (frame_type, _) in REMOVED_FRAMES {
                let frame = json!({ "type": frame_type, "id": "t1", "function_id": "api::run" });
                assert_eq!(
                    i.handle_downstream(&frame.to_string()).await,
                    DownstreamAction::Drop,
                    "{frame_type} must not reach the engine"
                );
                assert_eq!(
                    i.handle_upstream(&frame.to_string()).await,
                    UpstreamAction::Drop,
                    "{frame_type} must not reach the worker"
                );
            }
        }
        // One warning per frame type on this connection, however many came.
        assert_eq!(i.warned_removed.lock().await.len(), REMOVED_FRAMES.len());
    }

    #[tokio::test]
    async fn carve_out_channel_create_forwarded_even_with_empty_expose() {
        let i = interceptor(config_with(expose(&[]), Some("p::mw")), session(None));
        let frame = json!({"type":"invokefunction","invocation_id":"33333333-3333-3333-3333-333333333333","function_id":"engine::channels::create","data":{}});
        // Carve-out: allowed; engine:: bypasses middleware → forwarded, not
        // routed to middleware.
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(o["function_id"], "engine::channels::create");
    }

    // ----- features ----------------------------------------------------------

    fn register_with_features(features: &[&str]) -> String {
        json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": { "name": "w", "features": features },
            "action": { "type": "void" }
        })
        .to_string()
    }

    fn registered_with_features(features: &[&str]) -> String {
        json!({
            "type": "workerregistered",
            "worker_id": "7c1e0b52-93d4-4f0e-8a3b-6d2f1c9e4a70",
            "reattach_token": "t",
            "features": features
        })
        .to_string()
    }

    #[tokio::test]
    async fn observe_features_pass_and_local_dispatch_without_middleware() {
        let sess = session(None);
        let i = interceptor(config_with(expose(&[]), None), sess.clone());
        let o = forwarded(
            i.handle_downstream(&register_with_features(&[
                "observe.v1",
                "observe.v2",
                "local-dispatch.v1",
                "large-messages.v1",
            ]))
            .await,
        );
        assert_eq!(
            o["data"]["features"],
            json!(["observe.v1", "observe.v2", "local-dispatch.v1"])
        );
        assert_eq!(
            o["data"]["name"],
            format!("rbac-proxy:{}:w", sess.session_id)
        );

        let o = forwarded_up(
            i.handle_upstream(&registered_with_features(&[
                "observe.v1",
                "observe.v2",
                "local-dispatch.v1",
            ]))
            .await,
        );
        assert_eq!(
            o["features"],
            json!(["observe.v1", "observe.v2", "local-dispatch.v1"])
        );
        assert_eq!(o["reattach_token"], "t");
    }

    #[tokio::test]
    async fn under_middleware_local_dispatch_is_offered_for_own_private_functions_only() {
        let i = interceptor(
            config_with(expose(&[]), Some("p::mw")),
            session(Some("tenant1")),
        );
        let o = forwarded(
            i.handle_downstream(&register_with_features(&[
                "observe.v1",
                "local-dispatch.v1",
            ]))
            .await,
        );
        assert_eq!(
            o["data"]["features"],
            json!(["observe.v1", "local-dispatch.v1"])
        );

        let o = forwarded_up(
            i.handle_upstream(&registered_with_features(&["local-dispatch.v1"]))
                .await,
        );
        assert_eq!(o["features"], json!(["local-dispatch.v1"]));

        let ack = |id: &str, namespace: &str, local_dispatch: bool| {
            json!({
                "type": "functionregistered",
                "id": id,
                "namespace": namespace,
                "revision": 1,
                "local_dispatch": local_dispatch,
                "observers": []
            })
            .to_string()
        };

        // An ordinary own function: its own calls run on the wire, through
        // the middleware.
        register(&i, "ledger::post").await;
        let o = forwarded_up(
            i.handle_upstream(&ack("tenant1::ledger::post", "default", true))
                .await,
        );
        assert_eq!(o["id"], "ledger::post");
        assert_eq!(o["local_dispatch"], false);

        // A private one: its own calls skip the middleware, so they may run
        // in-process.
        let private =
            json!({ "type": "registerfunction", "id": "ledger::on-posted", "private": true });
        forwarded(i.handle_downstream(&private.to_string()).await);
        let o = forwarded_up(
            i.handle_upstream(&ack("tenant1::ledger::on-posted", "default", true))
                .await,
        );
        assert_eq!(o["id"], "ledger::on-posted");
        assert_eq!(o["local_dispatch"], true);

        // The engine's `false` stands.
        let o = forwarded_up(
            i.handle_upstream(&ack("tenant1::ledger::on-posted", "default", false))
                .await,
        );
        assert_eq!(o["local_dispatch"], false);

        // The same id in another namespace is not the connection's.
        let o = forwarded_up(
            i.handle_upstream(&ack("tenant1::ledger::on-posted", "orders", true))
                .await,
        );
        assert_eq!(o["local_dispatch"], false);

        // Registered again without `private`: an ordinary function.
        register(&i, "ledger::on-posted").await;
        let o = forwarded_up(
            i.handle_upstream(&ack("tenant1::ledger::on-posted", "default", true))
                .await,
        );
        assert_eq!(o["local_dispatch"], false);
    }

    // ----- observing ---------------------------------------------------------

    const OBSERVER_ID: &str = "5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d";

    fn observe(source: &str, target: &str) -> Value {
        json!({
            "type": "observe",
            "id": OBSERVER_ID,
            "source": { "function_id": source },
            "target": { "function_id": target }
        })
    }

    #[tokio::test]
    async fn observing_exposed_functions_forwards() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = observe("api::orders", "api::sink");
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(o, frame);
    }

    #[tokio::test]
    async fn an_observer_of_a_forbidden_function_or_target_is_refused() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        for frame in [
            observe("api::orders", "secret::run"),
            observe("secret::orders", "api::sink"),
        ] {
            let r = reply(i.handle_downstream(&frame.to_string()).await);
            assert_eq!(r["type"], "observeresult");
            assert_eq!(r["id"], OBSERVER_ID);
            assert_eq!(r["state"], "error");
            assert_eq!(r["error"]["code"], "FORBIDDEN");
            assert!(r["error"]["message"]
                .as_str()
                .unwrap()
                .contains("add to rbac.expose_functions"));
        }
    }

    #[tokio::test]
    async fn an_observer_of_own_functions_resolves_their_prefixed_ids() {
        // No expose patterns, but the session registered both functions.
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "foo").await;
        register(&i, "bar").await;
        let o = forwarded(
            i.handle_downstream(&observe("foo", "bar").to_string())
                .await,
        );
        assert_eq!(o["source"]["function_id"], "tenant1::foo");
        assert_eq!(o["target"]["function_id"], "tenant1::bar");
    }

    #[tokio::test]
    async fn an_observer_target_is_checked_in_its_namespace() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let mut frame = observe("api::orders", "api::run");
        frame["target"]["namespace"] = json!("orders");
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        frame["target"]["namespace"] = json!("default");
        forwarded(i.handle_downstream(&frame.to_string()).await);
    }

    #[tokio::test]
    async fn engine_targets_and_undisclosable_sources_are_refused() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        let r = reply(
            i.handle_downstream(&observe("api::orders", "engine::functions::list").to_string())
                .await,
        );
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        for source in [
            "engine::channels::create",
            "engine::log::info",
            "engine::functions::list",
            "engine::observers::dispose",
            "engine::on-functions-changed",
            "engine::on-workers-changed",
            "engine::on-observers-changed",
        ] {
            let r = reply(
                i.handle_downstream(&observe(source, "api::sink").to_string())
                    .await,
            );
            assert_eq!(r["error"]["code"], "FORBIDDEN", "{source}");
        }
    }

    #[tokio::test]
    async fn an_observe_the_engine_could_not_decode_is_refused_or_dropped() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        let mut frame = observe("api::orders", "api::sink");
        frame["source"]["function_id"] = json!(7);
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["error"]["code"], "INVALID_ARGS");

        let mut frame = observe("api::orders", "api::sink");
        frame["target"]["namespace"] = json!(5);
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["error"]["code"], "INVALID_ARGS");

        // Without a string id there is nothing to answer.
        let no_id = json!({ "type": "observe", "source": {}, "target": {} });
        assert_eq!(
            i.handle_downstream(&no_id.to_string()).await,
            DownstreamAction::Drop
        );
    }

    fn dispose(id: &str) -> String {
        json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "engine::observers::dispose",
            "data": { "id": id }
        })
        .to_string()
    }

    #[tokio::test]
    async fn only_observers_this_connection_created_can_be_disposed() {
        let i = interceptor(
            config_with(expose(&["api::*", "engine::observers::*"]), None),
            session(None),
        );
        // Not created yet (the engine has not accepted it).
        forwarded(
            i.handle_downstream(&observe("api::orders", "api::sink").to_string())
                .await,
        );
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        let accepted = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "pending" });
        forwarded_up(i.handle_upstream(&accepted.to_string()).await);
        forwarded(i.handle_downstream(&dispose(OBSERVER_ID)).await);

        // Somebody else's observer.
        let r = reply(
            i.handle_downstream(&dispose("0ad9be10-9c1b-425e-9cdd-74160cb9fe09"))
                .await,
        );
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        // Disposed: no longer this connection's.
        let disposed = json!({ "type": "observerstatechanged", "id": OBSERVER_ID, "state": "disposed", "fires": 3 });
        forwarded_up(i.handle_upstream(&disposed.to_string()).await);
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    /// A worker that disposes its observer observes again under the id at
    /// once. The engine answers both observes in order; the first answer and
    /// its disposal are the first observer's, and the second observer is the
    /// connection's once its own answer arrives.
    #[tokio::test]
    async fn an_id_observed_again_before_the_first_answer_belongs_to_the_second() {
        let i = interceptor(
            config_with(expose(&["api::*", "engine::observers::*"]), None),
            session(None),
        );
        let observe = observe("api::orders", "api::sink").to_string();
        let unobserve = json!({ "type": "unobserve", "id": OBSERVER_ID }).to_string();
        forwarded(i.handle_downstream(&observe).await);
        forwarded(i.handle_downstream(&unobserve).await);
        forwarded(i.handle_downstream(&observe).await);

        let answer = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "pending" });
        forwarded_up(i.handle_upstream(&answer.to_string()).await);
        let disposed = json!({ "type": "observerstatechanged", "id": OBSERVER_ID, "state": "disposed", "fires": 0 });
        forwarded_up(i.handle_upstream(&disposed.to_string()).await);
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        forwarded_up(i.handle_upstream(&answer.to_string()).await);
        forwarded(i.handle_downstream(&dispose(OBSERVER_ID)).await);
    }

    /// An observe the proxy refuses while an earlier observe under its id
    /// waits for the engine's answer: the refusal goes out right after that
    /// answer, in the order the worker sent the observes.
    #[tokio::test]
    async fn a_refusal_follows_the_answer_to_an_earlier_observe_under_its_id() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let refused = observe("api::orders", "engine::functions::list").to_string();
        forwarded(
            i.handle_downstream(&observe("api::orders", "api::sink").to_string())
                .await,
        );
        forwarded(
            i.handle_downstream(&json!({ "type": "unobserve", "id": OBSERVER_ID }).to_string())
                .await,
        );
        assert_eq!(i.handle_downstream(&refused).await, DownstreamAction::Drop);

        let answer = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "pending" });
        let UpstreamAction::ForwardAll(frames) = i.handle_upstream(&answer.to_string()).await
        else {
            panic!("expected the answer and the refusal");
        };
        let frames: Vec<Value> = frames
            .iter()
            .map(|f| serde_json::from_str(f).unwrap())
            .collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], answer);
        assert_eq!(frames[1]["id"], OBSERVER_ID);
        assert_eq!(frames[1]["state"], "error");
        assert_eq!(frames[1]["error"]["code"], "FORBIDDEN");

        // Nothing waits under the id: a refusal goes out at once.
        let r = reply(i.handle_downstream(&refused).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    /// The engine's refusal also unobserves the id first: a `Reattach` gives
    /// the connection its previous connection's observers with no RBAC check
    /// (the proxy's connections carry no session), and a replay the engine
    /// refuses (a spec the registration hook rewrote otherwise for this
    /// session) leaves that observer active as the earlier session had it.
    #[tokio::test]
    async fn a_refused_observer_is_not_created() {
        let i = interceptor(
            config_with(expose(&["api::*", "engine::observers::*"]), None),
            session(None),
        );
        forwarded(
            i.handle_downstream(&observe("api::orders", "api::sink").to_string())
                .await,
        );
        let refused = json!({
            "type": "observeresult",
            "id": OBSERVER_ID,
            "state": "error",
            "error": { "code": "OBSERVER_ID_CONFLICT", "message": "owned by another connection" }
        });
        assert_eq!(
            i.handle_upstream(&refused.to_string()).await,
            UpstreamAction::ReplyThenForwardAll(
                unobserve_frame(OBSERVER_ID),
                vec![refused.to_string()]
            )
        );
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    /// A refusal that a later observe of the id waits behind leaves the id
    /// alone: the engine would read an `unobserve` after that observe and
    /// dispose what it creates.
    #[tokio::test]
    async fn a_refusal_with_a_later_observe_of_its_id_waiting_does_not_unobserve() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let observe = observe("api::orders", "api::sink").to_string();
        forwarded(i.handle_downstream(&observe).await);
        forwarded(i.handle_downstream(&observe).await);
        assert_eq!(
            i.handle_upstream(&engine_refusal()).await,
            UpstreamAction::Forward(engine_refusal())
        );
    }

    fn engine_refusal() -> String {
        json!({
            "type": "observeresult",
            "id": OBSERVER_ID,
            "state": "error",
            "error": { "code": "OBSERVER_ID_CONFLICT", "message": "owned by another connection" }
        })
        .to_string()
    }

    /// The engine's refusal is its observe's final answer: it ends that
    /// observe's entry, and the next answer under the id is the next
    /// observe's.
    #[tokio::test]
    async fn a_refusal_ends_its_observe_and_the_next_answer_is_the_next_observes() {
        let i = interceptor(
            config_with(expose(&["api::*", "engine::observers::*"]), None),
            session(None),
        );
        let observe = observe("api::orders", "api::sink").to_string();
        forwarded(i.handle_downstream(&observe).await);
        assert!(matches!(
            i.handle_upstream(&engine_refusal()).await,
            UpstreamAction::ReplyThenForwardAll(..)
        ));
        assert!(!i.observers.lock().await.requested.contains_key(OBSERVER_ID));

        forwarded(i.handle_downstream(&observe).await);
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
        let accepted = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "active" });
        forwarded_up(i.handle_upstream(&accepted.to_string()).await);
        forwarded(i.handle_downstream(&dispose(OBSERVER_ID)).await);
    }

    /// A worker disposes an observer before the engine refuses it, and
    /// observes again under the id. The refusal is the first observe's, the
    /// `unobserve` between them disposes nothing (the engine holds nothing
    /// for a refused observe), and the second observe is the connection's
    /// once its own answer arrives, with its own `deliver`.
    #[tokio::test]
    async fn a_refusal_after_an_unobserve_leaves_the_next_answer_to_the_next_observe() {
        let i = interceptor(
            config_with(expose(&["api::*", "engine::observers::*"]), None),
            session(None),
        );
        let first = observe("api::orders", "api::sink");
        let mut second = first.clone();
        second["source"]["deliver"] = json!("input");
        let unobserve = json!({ "type": "unobserve", "id": OBSERVER_ID }).to_string();
        forwarded(i.handle_downstream(&first.to_string()).await);
        forwarded(i.handle_downstream(&unobserve).await);
        forwarded(i.handle_downstream(&second.to_string()).await);

        forwarded_up(i.handle_upstream(&engine_refusal()).await);
        let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        let accepted = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "active" });
        forwarded_up(i.handle_upstream(&accepted.to_string()).await);
        {
            let book = i.observers.lock().await;
            assert_eq!(book.created.get(OBSERVER_ID), Some(&false));
            assert!(!book.requested.contains_key(OBSERVER_ID));
        }
        forwarded(i.handle_downstream(&dispose(OBSERVER_ID)).await);
    }

    /// An `unobserve` that follows an `observe` to the engine disposes the
    /// observer that `observe` creates: its acceptance makes nothing this
    /// connection's, whether the engine's `disposed` for it arrives before or
    /// after it.
    #[tokio::test]
    async fn an_observe_unobserved_before_its_answer_is_not_created() {
        let accepted = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "active" });
        let disposed = json!({ "type": "observerstatechanged", "id": OBSERVER_ID, "state": "disposed", "fires": 0 });
        let unobserve = json!({ "type": "unobserve", "id": OBSERVER_ID }).to_string();
        for disposed_first in [true, false] {
            let i = interceptor(
                config_with(expose(&["api::*", "engine::observers::*"]), None),
                session(None),
            );
            forwarded(
                i.handle_downstream(&observe("api::orders", "api::sink").to_string())
                    .await,
            );
            forwarded(i.handle_downstream(&unobserve).await);
            if disposed_first {
                forwarded_up(i.handle_upstream(&disposed.to_string()).await);
            }
            forwarded_up(i.handle_upstream(&accepted.to_string()).await);
            let r = reply(i.handle_downstream(&dispose(OBSERVER_ID)).await);
            assert_eq!(
                r["error"]["code"], "FORBIDDEN",
                "disposed first: {disposed_first}"
            );
            assert!(i.observers.lock().await.created.is_empty());
        }
    }

    #[tokio::test]
    async fn observer_builtins_are_gated_like_discovery() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        for id in ["engine::observers::list", "engine::observers::get"] {
            let frame = json!({
                "type": "invokefunction",
                "invocation_id": "11111111-1111-1111-1111-111111111111",
                "function_id": id,
                "data": {}
            });
            let r = reply(i.handle_downstream(&frame.to_string()).await);
            assert_eq!(r["error"]["code"], "FORBIDDEN", "{id}");
        }

        let i = interceptor(config_with(expose(&["engine::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "engine::observers::list",
            "data": {}
        });
        forwarded(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(
            i.pending_overrides
                .lock()
                .await
                .values()
                .next()
                .map(String::as_str),
            Some("engine::observers::list")
        );
    }

    #[tokio::test]
    async fn an_envelope_of_an_own_observer_names_the_bare_function() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "orders").await;
        register(&i, "sink").await;
        forwarded(
            i.handle_downstream(&observe("orders", "sink").to_string())
                .await,
        );
        let accepted = json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "active" });
        forwarded_up(i.handle_upstream(&accepted.to_string()).await);

        let activation = json!({
            "type": "invokefunction",
            "invocation_id": "f1a3c5e7-9b1d-4f3a-8c5e-7a9b1d3f5a7c",
            "function_id": "tenant1::sink",
            "data": {
                "observer_id": OBSERVER_ID,
                "activation_id": "2d4f6a8c-0e2b-4d6f-9a1c-3e5a7c9e1b3d",
                "function_id": "tenant1::orders",
                "status": "success",
                "input": { "id": 1 }
            },
            "activation": { "observer_id": OBSERVER_ID, "activation_id": "2d4f6a8c-0e2b-4d6f-9a1c-3e5a7c9e1b3d" }
        });
        let o = forwarded_up(i.handle_upstream(&activation.to_string()).await);
        assert_eq!(o["function_id"], "sink");
        assert_eq!(o["data"]["function_id"], "orders");
        assert_eq!(o["data"]["input"], json!({ "id": 1 }));
    }

    // ----- hosting -----------------------------------------------------------

    fn function_registered(id: &str, observers: &[(&str, &str)]) -> String {
        json!({
            "type": "functionregistered",
            "id": id,
            "namespace": "default",
            "revision": 2,
            "local_dispatch": true,
            "observers": observers
                .iter()
                .map(|(observer_id, target)| json!({
                    "id": observer_id,
                    "args": {},
                    "filters": {},
                    "target": target,
                    "capture": ["input"],
                    "on": "success"
                }))
                .collect::<Vec<_>>()
        })
        .to_string()
    }

    fn call_through(observer_id: &str, invocation_id: Option<&str>) -> String {
        let mut frame = json!({
            "type": "invokefunction",
            "function_id": "users::get",
            "data": { "path": "/users/42" },
            "activation": { "observer_id": observer_id }
        });
        match invocation_id {
            Some(id) => frame["invocation_id"] = json!(id),
            None => frame["action"] = json!({ "type": "void" }),
        }
        frame.to_string()
    }

    #[tokio::test]
    async fn the_host_frames_of_own_functions_lose_the_prefix() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "cron::on-tick").await;

        let o = forwarded_up(
            i.handle_upstream(&function_registered(
                "tenant1::cron::on-tick",
                &[("o-1", "tenant1::handler"), ("o-2", "reports::refresh")],
            ))
            .await,
        );
        assert_eq!(o["id"], "cron::on-tick");
        assert_eq!(o["observers"][0]["target"], "handler");
        assert_eq!(o["observers"][1]["target"], "reports::refresh");
        assert_eq!(o["local_dispatch"], true);

        let added = json!({
            "type": "observeradded",
            "function_id": "tenant1::cron::on-tick",
            "namespace": "default",
            "revision": 3,
            "observer": { "id": "o-3", "args": {}, "filters": {}, "target": "tenant1::other", "capture": ["input"], "on": "success" }
        });
        let o = forwarded_up(i.handle_upstream(&added.to_string()).await);
        assert_eq!(o["function_id"], "cron::on-tick");
        assert_eq!(o["observer"]["target"], "other");

        let removed = json!({
            "type": "observerremoved",
            "function_id": "tenant1::cron::on-tick",
            "namespace": "default",
            "revision": 4,
            "id": "o-3"
        });
        let o = forwarded_up(i.handle_upstream(&removed.to_string()).await);
        assert_eq!(o["function_id"], "cron::on-tick");

        let hosted = i.observers.lock().await.hosted.clone();
        let mut ids: Vec<&str> = hosted.keys().map(String::as_str).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["o-1", "o-2"]);
    }

    #[tokio::test]
    async fn a_call_through_an_observer_needs_a_hosted_observer() {
        let i = interceptor(config_with(expose(&[]), Some("p::mw")), session(None));
        let id = "e7c9a1b3-d5f7-4a9b-8d1f-3b5d7f9a1c3e";

        let r = reply(i.handle_downstream(&call_through("o-1", Some(id))).await);
        assert_eq!(r["invocation_id"], id);
        assert_eq!(r["error"]["code"], "NOT_FUNCTION_OWNER");
        // Fire-and-forget: dropped, as the engine drops a refused one.
        assert_eq!(
            i.handle_downstream(&call_through("o-1", None)).await,
            DownstreamAction::Drop
        );

        register(&i, "http::on-request").await;
        forwarded_up(
            i.handle_upstream(&function_registered(
                "http::on-request",
                &[("o-1", "users::get")],
            ))
            .await,
        );
        // Forwarded as sent: no access check and no middleware (the engine
        // routes it to the observer's stored target).
        let o = forwarded(i.handle_downstream(&call_through("o-1", Some(id))).await);
        assert_eq!(o["function_id"], "users::get");
        assert_eq!(o["activation"]["observer_id"], "o-1");
        forwarded(i.handle_downstream(&call_through("o-1", None)).await);

        let removed = json!({ "type": "observerremoved", "function_id": "http::on-request", "namespace": "default", "revision": 3, "id": "o-1" });
        forwarded_up(i.handle_upstream(&removed.to_string()).await);
        let r = reply(i.handle_downstream(&call_through("o-1", Some(id))).await);
        assert_eq!(r["error"]["code"], "NOT_FUNCTION_OWNER");
    }

    #[tokio::test]
    async fn a_malformed_activation_is_an_ordinary_call() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "22222222-2222-2222-2222-222222222222",
            "function_id": "api::users::get",
            "data": {},
            "activation": { "observer_id": 7 }
        });
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert!(o.get("activation").is_none());

        // `null` is absent.
        let mut null = frame.clone();
        null["activation"] = Value::Null;
        forwarded(i.handle_downstream(&null.to_string()).await);

        // And still gated: a forbidden target is refused.
        let mut forbidden = frame.clone();
        forbidden["function_id"] = json!("secret::get");
        let r = reply(i.handle_downstream(&forbidden.to_string()).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn an_observerack_is_forwarded_for_own_functions_only() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "cron::on-tick").await;
        let ack = json!({
            "type": "observerack",
            "function_id": "cron::on-tick",
            "namespace": "default",
            "id": "a07d4c2e-6b1f-4a3d-9e8c-5f2a1b3c4d5e",
            "revision": 13
        });
        let o = forwarded(i.handle_downstream(&ack.to_string()).await);
        assert_eq!(o["function_id"], "tenant1::cron::on-tick");

        let mut foreign = ack.clone();
        foreign["function_id"] = json!("billing::charge");
        assert_eq!(
            i.handle_downstream(&foreign.to_string()).await,
            DownstreamAction::Drop
        );
        let mut elsewhere = ack.clone();
        elsewhere["namespace"] = json!("orders");
        assert_eq!(
            i.handle_downstream(&elsewhere.to_string()).await,
            DownstreamAction::Drop
        );
    }

    fn report_item(function_id: &str, invocation_id: &str, caller: Option<Value>) -> Value {
        let mut item = json!({
            "function_id": function_id,
            "namespace": "default",
            "invocation_id": invocation_id,
            "matched": ["c81e2f4a-5b6c-4d7e-9f0a-1b2c3d4e5f60"],
            "status": "success",
            "input": { "q": 1 },
            "output_is_input": true,
            "started_at_ms": 1,
            "duration_ms": 0.5,
            "observe_depth": 0
        });
        if let Some(caller) = caller {
            item["caller"] = caller;
        }
        item
    }

    /// The engine routes `orders::submit` (own) to this connection, called by
    /// `checkout::confirm` on the `checkout` worker.
    async fn route_submit(i: &Interceptor) {
        let routed = json!({
            "type": "invokefunction",
            "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
            "function_id": "tenant1::orders::submit",
            "data": { "sku": "A-1" },
            "caller": {
                "worker_id": "2b8d4f6a-1c3e-4a5b-9d7f-0e2c4a6b8d1f",
                "worker_name": "checkout",
                "function_id": "checkout::confirm",
                "invocation_id": "5e7a9c1b-2d4f-4b6a-8c0e-1f3a5c7e9b20",
                "parent": { "worker_name": "web", "function_id": "http::on-request", "invocation_id": "9b1d3f5a-7c9e-4b1d-8f3a-5c7e9b1d3f5a" }
            }
        });
        let o = forwarded_up(i.handle_upstream(&routed.to_string()).await);
        assert_eq!(o["function_id"], "orders::submit");
    }

    #[tokio::test]
    async fn a_caller_is_kept_only_for_a_call_routed_here() {
        let i = interceptor(
            config_with(expose(&["api::*"]), None),
            session(Some("tenant1")),
        );
        register(&i, "orders::submit").await;
        route_submit(&i).await;

        let call = |caller: Value| {
            json!({
                "type": "invokefunction",
                "invocation_id": "11111111-1111-1111-1111-111111111111",
                "function_id": "api::reserve",
                "data": {},
                "caller": caller,
                "observe_depth": 2
            })
            .to_string()
        };
        // Made inside the routed call: written from it.
        let o = forwarded(
            i.handle_downstream(&call(json!({
                "function_id": "orders::submit",
                "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
                "parent": { "worker_name": "billing", "function_id": "forged::fn" }
            })))
            .await,
        );
        assert_eq!(
            o["caller"],
            json!({
                "function_id": "tenant1::orders::submit",
                "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
                "parent": {
                    "worker_id": "2b8d4f6a-1c3e-4a5b-9d7f-0e2c4a6b8d1f",
                    "worker_name": "checkout",
                    "function_id": "checkout::confirm",
                    "invocation_id": "5e7a9c1b-2d4f-4b6a-8c0e-1f3a5c7e9b20"
                }
            })
        );
        assert_eq!(o["observe_depth"], 2);

        // Naming any other call: removed.
        let o = forwarded(
            i.handle_downstream(&call(json!({
                "function_id": "billing::charge",
                "invocation_id": "77e1c3a5-b7d9-4f1b-9d3f-5b7d9f1b3d5f",
                "parent": { "worker_name": "checkout" }
            })))
            .await,
        );
        assert!(o.get("caller").is_none());

        // Once answered, the routed call is no context any more.
        let answer = json!({
            "type": "invocationresult",
            "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
            "function_id": "orders::submit",
            "result": { "ok": true },
            "matched": ["c6ccda45-4a35-461a-a9c4-a318eec7c36a"]
        });
        let o = forwarded(i.handle_downstream(&answer.to_string()).await);
        assert_eq!(o["matched"], answer["matched"]);
        let o = forwarded(
            i.handle_downstream(&call(json!({
                "function_id": "orders::submit",
                "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a"
            })))
            .await,
        );
        assert!(o.get("caller").is_none());
    }

    #[tokio::test]
    async fn report_items_are_kept_for_own_functions_with_a_vouched_caller() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "orders::submit").await;
        register(&i, "orders::archive").await;
        route_submit(&i).await;

        let batch = json!({
            "type": "invocationsobserved",
            "items": [
                // A local call made inside the routed call: its caller is
                // written from that call.
                report_item(
                    "orders::archive",
                    "8a0c2e4f-6b8d-4f1a-9c3e-5a7c9e1b3d46",
                    Some(json!({
                        "function_id": "orders::submit",
                        "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
                        "parent": { "worker_name": "billing", "function_id": "forged::fn" }
                    }))
                ),
                // A local call naming a call that was not routed here: its
                // caller is removed.
                report_item(
                    "orders::submit",
                    "2f4a6c8e-0b1d-4f3a-8c5e-7a9c1e3b5d70",
                    Some(json!({ "function_id": "forged::fn", "invocation_id": "77e1c3a5-b7d9-4f1b-9d3f-5b7d9f1b3d5f" }))
                ),
                // A local call outside any handler.
                report_item("orders::archive", "4b6d8f0a-2c4e-4a6c-8e0a-2c4e6a8c0e1f", None),
                // Somebody else's function.
                report_item("payments::charge", "0d4b6f81-3a5c-4e7d-9b1f-2c4e6a8b0d13", None),
            ]
        });
        let o = forwarded(i.handle_downstream(&batch.to_string()).await);
        let items = o["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["function_id"], "tenant1::orders::archive");
        assert_eq!(
            items[0]["caller"],
            json!({
                "function_id": "tenant1::orders::submit",
                "invocation_id": "b414a098-c011-4e9e-8e65-e45bbe4e645a",
                "parent": {
                    "worker_id": "2b8d4f6a-1c3e-4a5b-9d7f-0e2c4a6b8d1f",
                    "worker_name": "checkout",
                    "function_id": "checkout::confirm",
                    "invocation_id": "5e7a9c1b-2d4f-4b6a-8c0e-1f3a5c7e9b20"
                }
            })
        );
        assert_eq!(items[0]["output_is_input"], true);
        assert_eq!(items[1]["function_id"], "tenant1::orders::submit");
        assert!(items[1].get("caller").is_none());
        assert_eq!(items[2]["function_id"], "tenant1::orders::archive");
        assert!(items[2].get("caller").is_none());

        // A batch of nothing but foreign items is dropped.
        let foreign = json!({
            "type": "invocationsobserved",
            "items": [report_item("payments::charge", "0d4b6f81-3a5c-4e7d-9b1f-2c4e6a8b0d13", None)]
        });
        assert_eq!(
            i.handle_downstream(&foreign.to_string()).await,
            DownstreamAction::Drop
        );
    }

    #[tokio::test]
    async fn local_invocation_stats_keep_own_functions() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        register(&i, "database::on-row-changed").await;
        let stats = json!({
            "type": "localinvocationstats",
            "window_ms": 10000,
            "functions": [
                { "function_id": "database::on-row-changed", "count": 3, "errors": 0, "duration_ms_sum": 1.5 },
                { "function_id": "billing::charge", "count": 1, "errors": 0, "duration_ms_sum": 0.5 }
            ],
            "dropped_reports": [
                { "function_id": "billing::charge", "reason": "BUFFER_FULL", "count": 2 }
            ]
        });
        let o = forwarded(i.handle_downstream(&stats.to_string()).await);
        assert_eq!(
            o["functions"],
            json!([{ "function_id": "tenant1::database::on-row-changed", "count": 3, "errors": 0, "duration_ms_sum": 1.5 }])
        );
        assert_eq!(o["dropped_reports"], json!([]));

        let foreign = json!({
            "type": "localinvocationstats",
            "window_ms": 10000,
            "functions": [{ "function_id": "billing::charge", "count": 1, "errors": 0, "duration_ms_sum": 0.5 }],
            "dropped_reports": []
        });
        assert_eq!(
            i.handle_downstream(&foreign.to_string()).await,
            DownstreamAction::Drop
        );
    }

    #[tokio::test]
    async fn known_frame_types_still_pass_through() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        for frame in [
            json!({ "type": "pong" }),
            json!({ "type": "unobserve", "id": OBSERVER_ID }),
            json!({ "type": "reattach", "previous_worker_id": "w", "reattach_token": "t" }),
            json!({ "type": "registerservice", "id": "s", "name": "s" }),
            json!({ "type": "invocationresult", "invocation_id": "11111111-1111-1111-1111-111111111111", "function_id": "f", "result": 1 }),
        ] {
            assert_eq!(
                i.handle_downstream(&frame.to_string()).await,
                DownstreamAction::Forward(frame.to_string())
            );
        }
        for frame in [
            json!({ "type": "registrationrejected", "code": "X", "namespace": "default", "owner_worker_id": "" }),
            json!({ "type": "observeresult", "id": OBSERVER_ID, "state": "active" }),
            json!({ "type": "observerstatechanged", "id": OBSERVER_ID, "state": "suspended", "fires": 1 }),
            json!({ "type": "error", "error": { "code": "AUTH_ERROR", "message": "m" } }),
            json!({ "type": "ping" }),
        ] {
            assert_eq!(
                i.handle_upstream(&frame.to_string()).await,
                UpstreamAction::Forward(frame.to_string())
            );
        }
    }

    // ----- namespaces ------------------------------------------------------

    fn scoped(prefix: Option<&str>, grants: &[(&str, &[&str])]) -> Arc<ProxySession> {
        Arc::new(ProxySession {
            function_registration_prefix: prefix.map(str::to_string),
            namespaces: grants
                .iter()
                .map(|(ns, ids)| (ns.to_string(), ids.iter().map(|s| s.to_string()).collect()))
                .collect(),
            ..ProxySession::permissive("1.2.3.4".to_string())
        })
    }

    fn invoke_in(namespace: Option<&str>, function_id: &str) -> String {
        let mut frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": function_id,
            "data": {}
        });
        if let Some(ns) = namespace {
            frame["namespace"] = json!(ns);
        }
        frame.to_string()
    }

    #[tokio::test]
    async fn an_expose_rule_without_namespace_answers_in_default_only() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        forwarded(i.handle_downstream(&invoke_in(None, "api::a")).await);
        forwarded(
            i.handle_downstream(&invoke_in(Some("default"), "api::a"))
                .await,
        );
        let r = reply(
            i.handle_downstream(&invoke_in(Some("orders"), "api::a"))
                .await,
        );
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn a_namespaced_expose_rule_answers_in_its_namespace_only() {
        let rbac = RbacConfig {
            expose_functions: vec![FunctionFilter::match_pattern("api::*").in_namespace("orders")],
            ..RbacConfig::default()
        };
        let i = interceptor(config_with(rbac, None), session(None));
        let o = forwarded(
            i.handle_downstream(&invoke_in(Some("orders"), "api::a"))
                .await,
        );
        assert_eq!(o["namespace"], "orders");
        let r = reply(i.handle_downstream(&invoke_in(None, "api::a")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn scoped_grants_answer_in_their_namespace() {
        let i = interceptor(
            config_with(expose(&[]), None),
            scoped(None, &[("orders", &["svc::*"])]),
        );
        forwarded(
            i.handle_downstream(&invoke_in(Some("orders"), "svc::get"))
                .await,
        );
        let r = reply(
            i.handle_downstream(&invoke_in(Some("billing"), "svc::get"))
                .await,
        );
        assert_eq!(r["error"]["code"], "FORBIDDEN");
        let r = reply(i.handle_downstream(&invoke_in(None, "svc::get")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn a_non_string_namespace_is_dropped() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        let mut frame: Value = serde_json::from_str(&invoke_in(None, "api::a")).unwrap();
        frame["namespace"] = json!(5);
        assert_eq!(
            i.handle_downstream(&frame.to_string()).await,
            DownstreamAction::Drop
        );
    }

    fn register_worker_frame(namespace: Option<&str>) -> String {
        let mut frame = json!({
            "type": "invokefunction",
            "invocation_id": "44444444-4444-4444-4444-444444444444",
            "function_id": "engine::workers::register",
            "data": { "name": "billing" }
        });
        if let Some(ns) = namespace {
            frame["data"]["namespace"] = json!(ns);
        }
        frame.to_string()
    }

    #[tokio::test]
    async fn declaring_a_namespace_outside_the_grants_closes_the_connection() {
        let i = interceptor(
            config_with(expose(&[]), None),
            scoped(None, &[("orders", &[]), ("billing", &[])]),
        );
        let DownstreamAction::Close(frames) = i
            .handle_downstream(&register_worker_frame(Some("analytics")))
            .await
        else {
            panic!("expected the connection to be closed");
        };
        let frames: Vec<Value> = frames
            .iter()
            .map(|f| serde_json::from_str(f).unwrap())
            .collect();
        assert_eq!(frames[0]["type"], "invocationresult");
        assert_eq!(
            frames[0]["invocation_id"],
            "44444444-4444-4444-4444-444444444444"
        );
        assert_eq!(frames[0]["error"]["code"], "FORBIDDEN");
        assert_eq!(
            frames[0]["error"]["message"],
            "namespace 'analytics' not allowed for this session (permitted: billing, orders)"
        );
        assert_eq!(frames[1]["type"], "registrationrejected");
        assert_eq!(frames[1]["code"], "NAMESPACE_FORBIDDEN");
        assert_eq!(frames[1]["namespace"], "analytics");
        assert_eq!(frames[1]["worker_name"], "billing");

        // No namespace is `default`, which is not granted either.
        assert!(matches!(
            i.handle_downstream(&register_worker_frame(None)).await,
            DownstreamAction::Close(_)
        ));
        // A granted one goes through.
        forwarded(
            i.handle_downstream(&register_worker_frame(Some("orders")))
                .await,
        );
    }

    #[tokio::test]
    async fn an_unscoped_session_declares_any_namespace() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let o = forwarded(
            i.handle_downstream(&register_worker_frame(Some("analytics")))
                .await,
        );
        assert_eq!(o["data"]["namespace"], "analytics");
    }

    #[tokio::test]
    async fn own_functions_are_own_only_in_the_declared_namespace() {
        // tenant1 registers `foo` (engine id tenant1::foo) from a connection
        // declared in `orders`.
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        forwarded(
            i.handle_downstream(&register_worker_frame(Some("orders")))
                .await,
        );
        register(&i, "foo").await;

        let o = forwarded(i.handle_downstream(&invoke_in(Some("orders"), "foo")).await);
        assert_eq!(o["function_id"], "tenant1::foo");

        // The same id in `default` is somebody else's, and not exposed.
        let r = reply(i.handle_downstream(&invoke_in(None, "foo")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
        let r = reply(i.handle_downstream(&invoke_in(None, "tenant1::foo")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");

        // Its host frames name the declared namespace.
        let ack =
            json!({ "type": "observerack", "function_id": "foo", "id": "o-1", "revision": 1 });
        let o = forwarded(i.handle_downstream(&ack.to_string()).await);
        assert_eq!(o["function_id"], "tenant1::foo");
    }
}
