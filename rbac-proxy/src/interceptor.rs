//! Per-frame interception: parse each JSON text frame, decide per `type`
//! whether to forward (possibly rewritten), answer it out-of-band, or drop it,
//! then re-serialize. The proxy never invents a wire format — it speaks the
//! iii worker protocol verbatim and forwards anything it does not recognise
//! unchanged, so protocol additions degrade to pass-through.
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
use crate::rbac::{self, ProxySession};

/// What the downstream→engine pump should do with a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DownstreamAction {
    /// Forward this (possibly rewritten) text frame to the engine.
    Forward(String),
    /// Answer the downstream worker directly; do **not** touch the engine
    /// (deny synthesis, registration-denial frame, middleware result).
    ReplyToClient(String),
    /// Silently drop (denied `RegisterFunction` — the engine does the same).
    Drop,
}

/// What the engine→downstream pump should do with a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamAction {
    Forward(String),
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

    /// Resolve a worker-supplied bare `function_id` to the id as it exists in
    /// the engine registry: a prefixed session's own registration
    /// (`{prefix}::id`) when it registered one, else the canonical (foreign)
    /// id. See spec *Prefix resolution*.
    async fn resolve_target(&self, function_id: &str) -> String {
        if self.prefix().is_some() {
            let candidate = self.apply_prefix(function_id);
            if self.registered_ids.lock().await.contains(&candidate) {
                return candidate;
            }
        }
        function_id.to_string()
    }

    /// Is `engine_id` a function this session registered on this connection?
    async fn is_own(&self, engine_id: &str) -> bool {
        self.registered_ids.lock().await.contains(engine_id)
    }

    /// Access-resolution against the live rbac config + this session. The
    /// target's registered metadata is fetched from the catalog **only** when
    /// the config has a metadata filter (a cold cache then fails closed for
    /// metadata filters); wildcard-only configs need no catalog round trip.
    async fn allowed(&self, engine_id: &str) -> bool {
        let metadata = if self.config.rbac.uses_metadata() {
            self.catalog.metadata_for(engine_id).await
        } else {
            None
        };
        rbac::access_allowed(
            Some(&self.config.rbac),
            &self.session,
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
        let Ok(mut v) = serde_json::from_str::<Value>(text) else {
            // Not JSON we can classify — forward unchanged (the engine logs &
            // drops malformed frames; the proxy stays transparent).
            return DownstreamAction::Forward(text.to_string());
        };
        let Some(ty) = v.get("type").and_then(Value::as_str).map(str::to_string) else {
            return DownstreamAction::Forward(text.to_string());
        };

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
            // InvocationResult correlation is by invocation_id; RegisterService
            // is engine-internal. All pass through unchanged.
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

        // 3. Resolve the engine target with own-vs-foreign prefix rules.
        let bound_fn = v
            .get("function_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let target = self.resolve_target(&bound_fn).await;

        // 4. Target access check — own registrations are exempt; otherwise the
        //    session must be allowed to invoke the bound function (trigger
        //    firing bypasses the invoke gate, so this is always-on hardening
        //    beyond the engine).
        let permitted = self.is_own(&target).await || self.allowed(&target).await;
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

        // Resolve own-vs-foreign target (engine:: ids are never own; resolve
        // is a no-op for them).
        let target = self.resolve_target(&function_id).await;

        // Access resolution (forbidden > allowed > carve-out > expose > deny).
        let permitted = self.is_own(&target).await || self.allowed(&target).await;
        if !permitted {
            return self.deny_invoke(&function_id, inv_id, &v);
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
            self.spawn_middleware(mw_id, target, &v);
            return DownstreamAction::Drop;
        }

        // No middleware: forward with the resolved target.
        v["function_id"] = Value::String(target);
        DownstreamAction::Forward(v.to_string())
    }

    /// Synthesize the engine's `FORBIDDEN` `InvocationResult` (engine/mod.rs:
    /// 911-937). The engine always replies — even for a `void` action, where
    /// it fabricates an `invocation_id`; the proxy mirrors that.
    fn deny_invoke(&self, function_id: &str, inv_id: Option<Uuid>, v: &Value) -> DownstreamAction {
        let id = inv_id.unwrap_or_else(Uuid::new_v4);
        let remediation = rbac::remediation(&self.session, function_id);
        let mut result = json!({
            "type": "invocationresult",
            "invocation_id": id.to_string(),
            "function_id": function_id,
            "error": {
                "code": "FORBIDDEN",
                "message": rbac::forbidden_message(function_id, remediation),
            },
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
    fn spawn_middleware(&self, mw_id: String, target: String, v: &Value) {
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
        });
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
            match iii
                .trigger(TriggerRequest {
                    function_id: mw_id,
                    payload: input,
                    action: None,
                    timeout_ms: None,
                })
                .await
            {
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
            return UpstreamAction::Forward(text.to_string());
        };
        let Some(ty) = v.get("type").and_then(Value::as_str).map(str::to_string) else {
            return UpstreamAction::Forward(text.to_string());
        };

        match ty.as_str() {
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
    use crate::rbac::{FunctionFilter, RbacConfig, WildcardPattern};

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
                .map(|p| FunctionFilter::Match(WildcardPattern::new(p)))
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
        let UpstreamAction::Forward(out) = i.handle_upstream(&frame.to_string()).await;
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "foo");
    }

    #[tokio::test]
    async fn upstream_trigger_result_strips_prefix() {
        let i = interceptor(config_with(expose(&[]), None), session(Some("tenant1")));
        let frame = json!({"type":"triggerregistrationresult","id":"t1","trigger_type":"cron","function_id":"tenant1::foo"});
        let UpstreamAction::Forward(out) = i.handle_upstream(&frame.to_string()).await;
        let o: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(o["function_id"], "foo");
    }

    #[tokio::test]
    async fn unknown_frame_passes_through_unchanged() {
        let i = interceptor(config_with(expose(&[]), None), session(None));
        let frame = json!({"type":"ping"});
        assert_eq!(
            i.handle_downstream(&frame.to_string()).await,
            DownstreamAction::Forward(frame.to_string())
        );
        let frame2 = json!({"type":"somefutureframe","x":1});
        let UpstreamAction::Forward(out) = i.handle_upstream(&frame2.to_string()).await;
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), frame2);
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
}
