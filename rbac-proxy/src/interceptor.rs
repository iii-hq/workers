//! Per-frame interception: parse each JSON text frame, decide per `type`
//! whether to forward (possibly rewritten), answer it out-of-band, or drop it,
//! then re-serialize. The proxy never invents a wire format — it speaks the
//! iii worker protocol verbatim.
//!
//! It forwards only the frame types it knows ([`DOWNSTREAM_TYPES`],
//! [`UPSTREAM_TYPES`]) and drops the rest. The engine behind the proxy runs
//! with no RBAC session of its own, so a frame type the proxy has not learned
//! to gate is a frame nothing gates: protocol additions are closed until the
//! proxy learns them. iii 0.25's observe messages are the case in point. The
//! proxy also advertises no protocol features in either direction
//! ([`SUPPORTED_FEATURES`]), so SDKs and the engine fall back to the 0.24
//! protocol, and it refuses `observe` outright.
//!
//! Frames are parsed into [`serde_json::Value`] (not a typed `Message` enum)
//! so unknown fields and unknown frame types survive a rewrite byte-for-byte —
//! the proxy only ever mutates the specific keys it must.
//!
//! The interceptor holds the connection's [`ProxySession`] (derived once at
//! the upgrade) and the [`WorkerConfig`] snapshot captured at the upgrade —
//! tuning changes apply to the *next* connection; an in-flight connection
//! keeps the boundaries it was admitted under (spec: *Hot reload*).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
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
    "registertrigger",
    "unregistertrigger",
    "registertriggertype",
    "unregistertriggertype",
    "triggerregistrationresult",
    "invokefunction",
    "invocationresult",
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
    "registertrigger",
    "unregistertrigger",
    "triggerregistrationresult",
    "error",
    "ping",
    "pong",
];

/// Protocol features (iii 0.25 `features` negotiation) the proxy can enforce
/// end to end. None yet: `observe.v1` and `local-dispatch.v1` need the
/// interception the proxy does not have, so they are removed from the
/// worker's declaration and from the engine's answer.
pub const SUPPORTED_FEATURES: &[&str] = &[];

/// The engine's observer builtins (`engine::observers::list|get|dispose`). The
/// engine filters them by the caller's session, and the proxy's connections
/// have none, so they are refused whatever `expose_functions` says.
const OBSERVER_FUNCTIONS_PREFIX: &str = "engine::observers::";

/// `RegistrationRejected.code` for a namespace the session may not declare.
const NAMESPACE_FORBIDDEN: &str = "NAMESPACE_FORBIDDEN";

/// Observe error code for what this proxy does not support.
const UNSUPPORTED: &str = "UNSUPPORTED";

/// What the downstream→engine pump should do with a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DownstreamAction {
    /// Forward this (possibly rewritten) text frame to the engine.
    Forward(String),
    /// Answer the downstream worker directly; do **not** touch the engine
    /// (deny synthesis, registration-denial frame, middleware result).
    ReplyToClient(String),
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
    /// A frame type the proxy does not forward.
    Drop,
}

/// Per-connection interceptor state, shared (behind `Arc`) by both pump
/// directions.
pub struct Interceptor {
    /// Control connection — used to invoke the operator's auth/middleware/hook
    /// functions and to drive the discovery caches.
    pub iii: Arc<IIIClient>,
    /// Config snapshot captured at the upgrade (rbac, middleware, leak knob).
    pub config: Arc<WorkerConfig>,
    /// Shared discovery caches (catalog + binding index) over the control
    /// connection.
    pub catalog: Arc<CatalogCache>,
    /// Boundaries derived for this connection by the auth function.
    pub session: Arc<ProxySession>,
    /// Engine ids (prefixed where applicable) this session registered on this
    /// connection — the own-vs-foreign discriminator for invoke / trigger
    /// targets.
    registered_ids: Mutex<HashSet<String>>,
    /// The namespace this connection declared on `engine::workers::register`
    /// (`None` until it does, which the engine treats as `default`). Its own
    /// registrations live there, so an id is "own" only in this namespace.
    connection_namespace: Mutex<Option<String>>,
    /// `invocation_id → engine:: discovery fn` for results to rewrite.
    pending_overrides: Mutex<HashMap<Uuid, String>>,
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
            connection_namespace: Mutex::new(None),
            pending_overrides: Mutex::new(HashMap::new()),
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
        match self
            .iii
            .trigger(TriggerRequest {
                function_id: hook_id.to_string(),
                payload: input,
                action: None,
                timeout_ms: None,
            })
            .await
        {
            Ok(v) if v.is_object() => Some(v),
            other => {
                tracing::warn!(hook = %hook_id, result = ?other, "registration hook denied (threw or returned non-object)");
                None
            }
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

        if ty == "observe" {
            return refuse_observe(&v);
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
                    v["id"] = Value::String(engine_id);
                }
                DownstreamAction::Forward(v.to_string())
            }
            "registertrigger" => self.on_register_trigger(v).await,
            "registertriggertype" => self.on_register_trigger_type(v).await,
            "invokefunction" => self.on_invoke(v).await,
            // `UnregisterTrigger.id` is a trigger-instance id (never prefixed);
            // `UnregisterTriggerType` only takes effect for the type's own
            // provider; InvocationResult correlation is by invocation_id;
            // RegisterService is engine-internal; `Reattach` carries the
            // connection's own token. All pass through unchanged.
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
        // (engine/mod.rs:1251-1282; the README "after prefix" wording is a doc
        // bug, the code applies the prefix after the hook).
        if let Some(hook_id) = &self.config.rbac.on_function_registration_function_id {
            let input = json!({
                "function_id": bare_id,
                "description": v.get("description"),
                "metadata": v.get("metadata"),
                "context": self.session.context,
            });
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
        v["id"] = Value::String(engine_id);
        strip_observe_descriptor(&mut v);
        DownstreamAction::Forward(v.to_string())
    }

    async fn on_register_trigger_type(&self, mut v: Value) -> DownstreamAction {
        if !self.session.allow_trigger_type_registration {
            return self.deny_trigger_registration(&v, "trigger type registration not allowed");
        }

        if let Some(hook_id) = &self.config.rbac.on_trigger_type_registration_function_id {
            let input = json!({
                "trigger_type_id": v.get("id"),
                "description": v.get("description"),
                "context": self.session.context,
            });
            match self.call_hook(hook_id, input).await {
                Some(mapped) => {
                    if let Some(s) = mapped.get("trigger_type_id").and_then(Value::as_str) {
                        v["id"] = Value::String(s.to_string());
                    }
                    if let Some(s) = mapped.get("description").and_then(Value::as_str) {
                        v["description"] = Value::String(s.to_string());
                    }
                }
                None => return self.deny_trigger_registration(&v, "denied by hook"),
            }
        }

        DownstreamAction::Forward(v.to_string())
    }

    async fn on_register_trigger(&self, mut v: Value) -> DownstreamAction {
        let trigger_type = v
            .get("trigger_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 1. allowed_trigger_types (worker-supplied bare ids).
        if let Some(allowed) = &self.session.allowed_trigger_types {
            if !allowed.iter().any(|t| t == &trigger_type) {
                return self.deny_trigger_registration(&v, "trigger type not allowed");
            }
        }

        // 2. on_trigger_registration hook (bare ids), can remap.
        if let Some(hook_id) = &self.config.rbac.on_trigger_registration_function_id {
            let input = json!({
                "trigger_id": v.get("id"),
                "trigger_type": v.get("trigger_type"),
                "function_id": v.get("function_id"),
                "config": v.get("config"),
                "metadata": v.get("metadata"),
                "context": self.session.context,
            });
            match self.call_hook(hook_id, input).await {
                Some(mapped) => apply_trigger_hook_mapping(&mut v, &mapped),
                None => return self.deny_trigger_registration(&v, "denied by hook"),
            }
        }

        // 3. Resolve the engine target with own-vs-foreign prefix rules, in the
        //    namespace the trigger's target resolves in (`namespace`, absent =
        //    `default` whatever the registering connection declared).
        let bound_fn = v
            .get("function_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(namespace) = requested_namespace(&v) else {
            return self.deny_trigger_registration(&v, "namespace must be a string");
        };
        let target = self.resolve_target(&namespace, &bound_fn).await;

        // 4. Target access check — own registrations are exempt; otherwise the
        //    session must be allowed to invoke the bound function (trigger
        //    firing bypasses the invoke gate, so this is always-on hardening
        //    beyond the engine).
        let permitted =
            self.is_own(&namespace, &target).await || self.allowed(&namespace, &target).await;
        if !permitted {
            let remediation = rbac::remediation(&self.session, &target);
            let msg = rbac::forbidden_message(&target, remediation);
            return self.deny_trigger_registration(&v, &msg);
        }

        // 5. Forward with the resolved engine target.
        v["function_id"] = Value::String(target);
        DownstreamAction::Forward(v.to_string())
    }

    /// Build a `TriggerRegistrationResult{ error: REGISTRATION_DENIED }` for
    /// the worker. The engine denies silently; the proxy is deliberately more
    /// informative (intentional divergence). The worker sees the **bare** ids
    /// it sent.
    fn deny_trigger_registration(&self, v: &Value, message: &str) -> DownstreamAction {
        let id = v.get("id").and_then(Value::as_str).unwrap_or("");
        let trigger_type = v.get("trigger_type").and_then(Value::as_str).unwrap_or("");
        let function_id = v.get("function_id").and_then(Value::as_str).unwrap_or("");
        let frame = json!({
            "type": "triggerregistrationresult",
            "id": id,
            "trigger_type": trigger_type,
            "function_id": function_id,
            "error": { "code": "REGISTRATION_DENIED", "message": message },
        });
        tracing::debug!(
            id,
            trigger_type,
            function_id,
            message,
            "trigger registration denied"
        );
        DownstreamAction::ReplyToClient(frame.to_string())
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

        // iii 0.25 observe fields. `activation` marks an owner delivery
        // (`obs.call`), which the engine routes without a session; the proxy
        // does not support it yet, so it is refused rather than forwarded.
        // `caller` and `observe_depth` are the SDK's own assertions about the
        // call chain, which observers read; the proxy cannot vouch for them,
        // so they are removed.
        if v.get("activation").is_some_and(|a| !a.is_null()) {
            return self.reply_invoke_error(
                &function_id,
                inv_id,
                &v,
                UNSUPPORTED,
                "owner delivery (`activation`) is not supported through rbac-proxy",
            );
        }
        if let Some(obj) = v.as_object_mut() {
            obj.remove("caller");
            obj.remove("observe_depth");
        }

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

        if target.starts_with(OBSERVER_FUNCTIONS_PREFIX) {
            return self.reply_invoke_error(
                &function_id,
                inv_id,
                &v,
                "FORBIDDEN",
                &format!(
                    "function '{function_id}' is not available through rbac-proxy (observers are not supported yet)"
                ),
            );
        }

        // Access resolution (forbidden > scoped grant > allowed > carve-out >
        // expose > deny), in the call's namespace.
        let permitted =
            self.is_own(&namespace, &target).await || self.allowed(&namespace, &target).await;
        if !permitted {
            return self.deny_invoke(&function_id, inv_id, &v);
        }

        if target == "engine::workers::register" {
            return self.on_register_worker(v, inv_id).await;
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

    /// `engine::workers::register`, already allowed (it is in the carve-out).
    ///
    /// The namespace a connection declares is an authorization decision: once
    /// the auth function scopes the session to a set of namespaces, declaring
    /// one outside them is refused exactly as the engine refuses it — the
    /// call's `FORBIDDEN` result, a `RegistrationRejected { code:
    /// NAMESPACE_FORBIDDEN }`, then Close, so the worker cannot keep running
    /// (and registering) in `default`. An unscoped session declares freely.
    ///
    /// The worker's `features` are reduced to [`SUPPORTED_FEATURES`] on the
    /// way, so the engine treats it as a pre-0.25 worker.
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
            let mut request = TriggerRequest {
                function_id: mw_id,
                payload: input,
                action: None,
                timeout_ms: None,
            }
            .namespace(namespace);
            if let Some(metadata) = metadata {
                request = request.metadata(metadata);
            }
            match iii.trigger(request).await {
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
            // Engine dispatching a call TO the worker: strip the session's own
            // prefix so the worker SDK finds its local (bare) handler
            // (worker_connections/traits.rs:105-117).
            "invokefunction" => {
                if let Some(fid) = v.get("function_id").and_then(Value::as_str) {
                    v["function_id"] = Value::String(self.strip_prefix(fid));
                }
                UpstreamAction::Forward(v.to_string())
            }
            // Result of a call the worker made: rewrite if it is a recorded
            // discovery override; else pass through.
            "invocationresult" => self.on_invocation_result(v).await,
            // Strip the prefix so the worker sees the id it sent.
            "triggerregistrationresult" => {
                if let Some(fid) = v.get("function_id").and_then(Value::as_str) {
                    v["function_id"] = Value::String(self.strip_prefix(fid));
                }
                UpstreamAction::Forward(v.to_string())
            }
            _ => UpstreamAction::Forward(v.to_string()),
        }
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

        match engine_overrides::filter_result(
            &function_id,
            result,
            &self.session,
            &self.config,
            &self.catalog,
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

/// `observe` (iii 0.25) is refused: `ObserveResult { state: "error", error:
/// UNSUPPORTED }`, the answer an SDK gets for a feature the engine lacks. The
/// proxy cannot gate the observed function, the target, or the owner side of
/// an observer yet, and the engine behind it checks none of them for a
/// connection without a session.
fn refuse_observe(v: &Value) -> DownstreamAction {
    let Some(id) = v.get("id").and_then(Value::as_str) else {
        return DownstreamAction::Drop;
    };
    tracing::debug!(observer_id = %id, "refusing observe (not supported through rbac-proxy)");
    DownstreamAction::ReplyToClient(
        json!({
            "type": "observeresult",
            "id": id,
            "state": "error",
            "error": {
                "code": UNSUPPORTED,
                "message": "observers are not supported through rbac-proxy",
            },
        })
        .to_string(),
    )
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

/// Keep only the [`SUPPORTED_FEATURES`] in an object's `features` list, and
/// drop the key when none is left (an absent list reads as "no features" on
/// both sides).
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

/// `RegisterFunction.observe` (iii 0.25) declares how the function is
/// observed: owner delivery, an args schema, and a `legacy_trigger_type` that
/// makes it the stand-in for an old trigger type. None of it is gated by the
/// proxy yet (a stand-in would take over another provider's trigger type
/// without `allow_trigger_type_registration`), so the descriptor is removed.
/// Only an opt-out (`allow: false`) is kept: it narrows, never widens.
fn strip_observe_descriptor(v: &mut Value) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    if let Some(descriptor) = obj.remove("observe") {
        if descriptor.get("allow") == Some(&Value::Bool(false)) {
            obj.insert("observe".to_string(), json!({ "allow": false }));
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
/// description?, metadata? }` mapping onto the frame (omitted ⇒ unchanged).
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
}

/// Apply an `on_trigger_registration` hook's optional `{ trigger_id?,
/// trigger_type?, function_id?, config? }` mapping onto the frame.
fn apply_trigger_hook_mapping(v: &mut Value, mapped: &Value) {
    if let Some(s) = mapped.get("trigger_id").and_then(Value::as_str) {
        v["id"] = Value::String(s.to_string());
    }
    if let Some(s) = mapped.get("trigger_type").and_then(Value::as_str) {
        v["trigger_type"] = Value::String(s.to_string());
    }
    if let Some(s) = mapped.get("function_id").and_then(Value::as_str) {
        v["function_id"] = Value::String(s.to_string());
    }
    if let Some(c) = mapped.get("config") {
        v["config"] = c.clone();
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

    #[tokio::test]
    async fn denied_invoke_synthesizes_forbidden_with_echoed_id() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "secret::do",
            "data": {}
        });
        let action = i.handle_downstream(&frame.to_string()).await;
        let DownstreamAction::ReplyToClient(reply) = action else {
            panic!("expected a synthesized reply, got {action:?}");
        };
        let r: Value = serde_json::from_str(&reply).unwrap();
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
    async fn denied_void_invoke_fabricates_invocation_id() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": null,
            "function_id": "secret::do",
            "data": {},
            "action": { "type": "void" }
        });
        let DownstreamAction::ReplyToClient(reply) = i.handle_downstream(&frame.to_string()).await
        else {
            panic!("expected synthesized reply");
        };
        let r: Value = serde_json::from_str(&reply).unwrap();
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
        let DownstreamAction::ReplyToClient(reply) = i.handle_downstream(&frame.to_string()).await
        else {
            panic!("expected reply");
        };
        let r: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(
            r["error"]["message"],
            "function 'api::danger' not allowed (remove from rbac.forbidden_functions)"
        );
    }

    #[tokio::test]
    async fn allowed_invoke_forwards_unchanged_without_prefix() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"api::users::list","data":{"q":1}});
        let DownstreamAction::Forward(out) = i.handle_downstream(&frame.to_string()).await else {
            panic!("expected forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "api::users::list");
        assert_eq!(o["data"]["q"], 1);
    }

    #[tokio::test]
    async fn register_function_applies_prefix_and_tracks_own_id() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"registerfunction","id":"foo","request_format":null,"response_format":null});
        let DownstreamAction::Forward(out) = i.handle_downstream(&frame.to_string()).await else {
            panic!("expected forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["id"], "tenant1::foo");
        assert!(i.registered_ids.lock().await.contains("tenant1::foo"));
    }

    #[tokio::test]
    async fn prefixed_session_self_invokes_own_bare_function() {
        // Register `foo` (stored tenant1::foo), then invoke bare `foo`.
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let reg = json!({"type":"registerfunction","id":"foo","request_format":null,"response_format":null});
        let _ = i.handle_downstream(&reg.to_string()).await;

        let inv = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"foo","data":{}});
        let DownstreamAction::Forward(out) = i.handle_downstream(&inv.to_string()).await else {
            panic!("own-id self-invoke should be allowed + forwarded");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        // Resolved to the engine id.
        assert_eq!(o["function_id"], "tenant1::foo");
    }

    #[tokio::test]
    async fn foreign_invoke_matches_expose_unprefixed() {
        let i = interceptor(
            config_with(expose(&["api::*"]), None),
            session(Some("tenant1")),
        );
        let inv = json!({"type":"invokefunction","invocation_id":"11111111-1111-1111-1111-111111111111","function_id":"api::users::list","data":{}});
        let DownstreamAction::Forward(out) = i.handle_downstream(&inv.to_string()).await else {
            panic!("foreign exposed call should forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
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
    async fn trigger_type_not_allowed_replies_registration_denied() {
        let mut s = ProxySession::permissive("ip".to_string());
        s.allowed_trigger_types = Some(vec!["http".to_string()]);
        let i = interceptor(config_with(expose(&["*"]), None), Arc::new(s));
        let frame = json!({"type":"registertrigger","id":"t1","trigger_type":"cron","function_id":"api::run","config":{}});
        let DownstreamAction::ReplyToClient(reply) = i.handle_downstream(&frame.to_string()).await
        else {
            panic!("expected registration-denied reply");
        };
        let r: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(r["type"], "triggerregistrationresult");
        assert_eq!(r["error"]["code"], "REGISTRATION_DENIED");
    }

    #[tokio::test]
    async fn trigger_to_forbidden_function_is_denied() {
        // expose api::* only; bind a trigger to secret::run → denied.
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({"type":"registertrigger","id":"t1","trigger_type":"cron","function_id":"secret::run","config":{}});
        let DownstreamAction::ReplyToClient(reply) = i.handle_downstream(&frame.to_string()).await
        else {
            panic!("expected denial");
        };
        let r: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(r["error"]["code"], "REGISTRATION_DENIED");
        assert!(r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("add to rbac.expose_functions"));
    }

    #[tokio::test]
    async fn trigger_to_exposed_function_forwards_resolved_target() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({"type":"registertrigger","id":"t1","trigger_type":"cron","function_id":"api::run","config":{}});
        let DownstreamAction::Forward(out) = i.handle_downstream(&frame.to_string()).await else {
            panic!("expected forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "api::run");
    }

    #[tokio::test]
    async fn trigger_to_own_registered_function_allowed_even_if_not_exposed() {
        // No expose patterns, but the session registered `foo` (tenant1::foo);
        // binding a trigger to its own handler succeeds.
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let reg = json!({"type":"registerfunction","id":"foo","request_format":null,"response_format":null});
        let _ = i.handle_downstream(&reg.to_string()).await;
        let frame = json!({"type":"registertrigger","id":"t1","trigger_type":"cron","function_id":"foo","config":{}});
        let DownstreamAction::Forward(out) = i.handle_downstream(&frame.to_string()).await else {
            panic!("own-function trigger binding should be allowed");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "tenant1::foo");
    }

    #[tokio::test]
    async fn upstream_dispatch_strips_prefix() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"invokefunction","invocation_id":"22222222-2222-2222-2222-222222222222","function_id":"tenant1::foo","data":{}});
        let UpstreamAction::Forward(out) = i.handle_upstream(&frame.to_string()).await else {
            panic!("expected forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "foo");
    }

    #[tokio::test]
    async fn upstream_trigger_result_strips_prefix() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"triggerregistrationresult","id":"t1","trigger_type":"cron","function_id":"tenant1::foo"});
        let UpstreamAction::Forward(out) = i.handle_upstream(&frame.to_string()).await else {
            panic!("expected forward");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
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
    async fn carve_out_channel_create_forwarded_even_with_empty_expose() {
        let i = interceptor(config_with(expose(&[]), Some("p::mw")), session(None));
        let frame = json!({"type":"invokefunction","invocation_id":"33333333-3333-3333-3333-333333333333","function_id":"engine::channels::create","data":{}});
        // Carve-out: allowed; engine:: bypasses middleware → forwarded, not
        // routed to middleware.
        let DownstreamAction::Forward(out) = i.handle_downstream(&frame.to_string()).await else {
            panic!("carve-out call should be forwarded, not middleware-wrapped");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "engine::channels::create");
    }

    // ----- iii 0.25 observe: closed until the proxy can gate it -----------

    fn reply(action: DownstreamAction) -> Value {
        let DownstreamAction::ReplyToClient(text) = action else {
            panic!("expected a synthesized reply, got {action:?}");
        };
        serde_json::from_str(&text).unwrap()
    }

    fn forwarded(action: DownstreamAction) -> Value {
        let DownstreamAction::Forward(text) = action else {
            panic!("expected a forward, got {action:?}");
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn observe_is_refused_with_unsupported() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        let frame = json!({
            "type": "observe",
            "id": "5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d",
            "source": { "function_id": "api::orders" },
            "target": { "function_id": "api::sink" }
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["type"], "observeresult");
        assert_eq!(r["id"], "5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d");
        assert_eq!(r["state"], "error");
        assert_eq!(r["error"]["code"], "UNSUPPORTED");

        // Without a string id there is nothing to answer.
        let no_id = json!({ "type": "observe", "source": {}, "target": {} });
        assert_eq!(
            i.handle_downstream(&no_id.to_string()).await,
            DownstreamAction::Drop
        );
    }

    #[tokio::test]
    async fn observe_owner_frames_are_dropped() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        for ty in [
            "unobserve",
            "observerack",
            "invocationsobserved",
            "localinvocationstats",
        ] {
            let frame = json!({ "type": ty, "id": "x", "function_id": "api::a" });
            assert_eq!(
                i.handle_downstream(&frame.to_string()).await,
                DownstreamAction::Drop,
                "{ty} must not reach the engine"
            );
        }
    }

    #[tokio::test]
    async fn owner_delivery_invoke_is_refused() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "api::users::get",
            "data": {},
            "activation": { "observer_id": "o-1" }
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["type"], "invocationresult");
        assert_eq!(r["invocation_id"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(r["error"]["code"], "UNSUPPORTED");

        // Malformed (the engine would read it as absent): still refused, never
        // forwarded as an ordinary call carrying the field.
        let malformed = json!({
            "type": "invokefunction",
            "invocation_id": "22222222-2222-2222-2222-222222222222",
            "function_id": "api::users::get",
            "data": {},
            "activation": { "observer_id": 7 }
        });
        let r = reply(i.handle_downstream(&malformed.to_string()).await);
        assert_eq!(r["error"]["code"], "UNSUPPORTED");

        // `null` is absent: an ordinary call.
        let null = json!({
            "type": "invokefunction",
            "invocation_id": "33333333-3333-3333-3333-333333333333",
            "function_id": "api::users::get",
            "data": {},
            "activation": null
        });
        forwarded(i.handle_downstream(&null.to_string()).await);
    }

    #[tokio::test]
    async fn caller_and_observe_depth_are_removed_from_calls() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "invokefunction",
            "invocation_id": "11111111-1111-1111-1111-111111111111",
            "function_id": "api::a",
            "data": { "q": 1 },
            "caller": { "function_id": "billing::charge", "parent": { "worker_name": "checkout" } },
            "observe_depth": 3
        });
        let o = forwarded(i.handle_downstream(&frame.to_string()).await);
        assert!(o.get("caller").is_none());
        assert!(o.get("observe_depth").is_none());
        assert_eq!(o["data"]["q"], 1);
    }

    #[tokio::test]
    async fn features_are_removed_in_both_directions() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let register = json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": { "name": "w", "features": ["observe.v1", "local-dispatch.v1"] },
            "action": { "type": "void" }
        });
        let o = forwarded(i.handle_downstream(&register.to_string()).await);
        assert!(o["data"].get("features").is_none());
        assert_eq!(o["data"]["name"], "w");

        let registered = json!({
            "type": "workerregistered",
            "worker_id": "7c1e0b52-93d4-4f0e-8a3b-6d2f1c9e4a70",
            "reattach_token": "t",
            "features": ["observe.v1", "local-dispatch.v1"]
        });
        let UpstreamAction::Forward(out) = i.handle_upstream(&registered.to_string()).await else {
            panic!("workerregistered must be forwarded");
        };
        let o: Value = serde_json::from_str(&out).unwrap();
        assert!(o.get("features").is_none());
        assert_eq!(o["reattach_token"], "t");
    }

    #[tokio::test]
    async fn observe_engine_frames_are_dropped() {
        let i = interceptor(config_with(expose(&["*"]), None), session(None));
        for ty in [
            "functionregistered",
            "observeradded",
            "observerremoved",
            "observeresult",
            "observerstatechanged",
        ] {
            let frame = json!({ "type": ty, "id": "x", "function_id": "api::a" });
            assert_eq!(
                i.handle_upstream(&frame.to_string()).await,
                UpstreamAction::Drop,
                "{ty} must not reach the worker"
            );
        }
    }

    #[tokio::test]
    async fn observer_builtins_are_refused_even_when_exposed() {
        let i = interceptor(config_with(expose(&["engine::*"]), None), session(None));
        for id in [
            "engine::observers::list",
            "engine::observers::get",
            "engine::observers::dispose",
        ] {
            let frame = json!({
                "type": "invokefunction",
                "invocation_id": "11111111-1111-1111-1111-111111111111",
                "function_id": id,
                "data": {}
            });
            let r = reply(i.handle_downstream(&frame.to_string()).await);
            assert_eq!(r["error"]["code"], "FORBIDDEN", "{id}");
        }
    }

    #[tokio::test]
    async fn observe_descriptor_is_removed_from_registrations() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let stand_in = json!({
            "type": "registerfunction",
            "id": "my::cron",
            "observe": { "delivery": "owner", "legacy_trigger_type": "cron" }
        });
        let o = forwarded(i.handle_downstream(&stand_in.to_string()).await);
        assert!(o.get("observe").is_none());

        let opt_out = json!({
            "type": "registerfunction",
            "id": "my::private",
            "observe": { "allow": false, "legacy_trigger_type": "cron" }
        });
        let o = forwarded(i.handle_downstream(&opt_out.to_string()).await);
        assert_eq!(o["observe"], json!({ "allow": false }));
    }

    #[tokio::test]
    async fn known_frame_types_still_pass_through() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        for frame in [
            json!({ "type": "pong" }),
            json!({ "type": "unregistertrigger", "id": "t1" }),
            json!({ "type": "unregistertriggertype", "id": "cron" }),
            json!({ "type": "reattach", "previous_worker_id": "w", "reattach_token": "t" }),
            json!({ "type": "registerservice", "id": "s", "name": "s" }),
            json!({ "type": "triggerregistrationresult", "id": "t1", "trigger_type": "cron", "function_id": "f" }),
            json!({ "type": "invocationresult", "invocation_id": "11111111-1111-1111-1111-111111111111", "function_id": "f", "result": 1 }),
        ] {
            assert_eq!(
                i.handle_downstream(&frame.to_string()).await,
                DownstreamAction::Forward(frame.to_string())
            );
        }
        for frame in [
            json!({ "type": "registrationrejected", "code": "X", "namespace": "default", "owner_worker_id": "" }),
            json!({ "type": "registertrigger", "id": "t1", "trigger_type": "cron", "function_id": "f", "config": {} }),
            json!({ "type": "unregistertrigger", "id": "t1" }),
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
        let reg = json!({"type":"registerfunction","id":"foo"});
        forwarded(i.handle_downstream(&reg.to_string()).await);

        let o = forwarded(i.handle_downstream(&invoke_in(Some("orders"), "foo")).await);
        assert_eq!(o["function_id"], "tenant1::foo");

        // The same id in `default` is somebody else's, and not exposed.
        let r = reply(i.handle_downstream(&invoke_in(None, "foo")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
        let r = reply(i.handle_downstream(&invoke_in(None, "tenant1::foo")).await);
        assert_eq!(r["error"]["code"], "FORBIDDEN");
    }

    #[tokio::test]
    async fn a_trigger_target_is_checked_in_its_namespace() {
        let i = interceptor(config_with(expose(&["api::*"]), None), session(None));
        let frame = json!({
            "type": "registertrigger",
            "id": "t1",
            "trigger_type": "cron",
            "function_id": "api::run",
            "config": {},
            "namespace": "orders"
        });
        let r = reply(i.handle_downstream(&frame.to_string()).await);
        assert_eq!(r["type"], "triggerregistrationresult");
        assert_eq!(r["error"]["code"], "REGISTRATION_DENIED");

        let mut in_default = frame.clone();
        in_default.as_object_mut().unwrap().remove("namespace");
        forwarded(i.handle_downstream(&in_default.to_string()).await);
    }
}
