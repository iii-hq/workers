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

pub trait Engine: Send + Sync + 'static {
    /// Invoke any engine function. Unrestricted by design — the deployment's
    /// `iii-permissions.yaml` is the gate (see the spec's accepted risks).
    ///
    /// `action` routes the call the way the iii-sdk's own `trigger()` does
    /// (omit for plain request/response). It crosses as a string because the
    /// isolate boundary has exactly one representation to reason about — see
    /// `op_iii_call`, whose `iii.trigger` caller is what sends a non-`None`
    /// value.
    fn call(
        &self,
        fn_id: String,
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
    /// they get here (`wire::register::validate_format`).
    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        request_format: Option<Value>,
        response_format: Option<Value>,
        handler: ProxyHandler,
    ) -> UnregisterFn;

    /// Register a trigger on the caller's behalf. Returns the trigger id,
    /// which is what `unregister_trigger` needs.
    fn register_trigger(&self, input: Value) -> BoxFuture<'static, Result<String, String>>;

    /// Remove a trigger by the id `register_trigger` returned.
    fn unregister_trigger(&self, trigger_id: String) -> BoxFuture<'static, Result<(), String>>;

    /// Publish a trigger TYPE. The engine calls `handler` when a trigger of
    /// this type is registered or unregistered; the payload carries a
    /// `method` field naming which. The returned closure removes the type.
    fn register_trigger_type(
        &self,
        type_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn;
}

/// Stands in when a caller registers without a description, on node-engine's
/// own connection.
pub const DEFAULT_DYNAMIC_DESC: &str =
    "Registered at runtime by node-engine; implemented in JavaScript.";

/// A trigger registration as the GUEST writes it, which is the **node** sdk's
/// shape — its `type` field is what `iii.registerTrigger` documents and what
/// `prelude.js` validates.
///
/// The Rust sdk spells that same field `trigger_type`
/// (`iii_sdk::protocol::RegisterTriggerInput`) and carries no serde alias, so
/// the two ends of this worker's own boundary disagree on one key. Do NOT
/// "simplify" this back into `serde_json::from_value::<RegisterTriggerInput>`
/// on the guest's object: that deserialized the node-shaped input against the
/// Rust struct and made `iii.registerTrigger` fail for EVERY documented call
/// with ``missing field `trigger_type` `` — a defect no unit test could see,
/// because `FakeEngine` took a bare `Value` and never deserialized at all.
///
/// This type is also why a malformed registration is reported in the guest's
/// own vocabulary: a missing `type` says `type`, not `trigger_type`.
/// Deliberately does NOT accept `trigger_type` — the guest contract is
/// node-shaped, and one field must have one spelling.
#[derive(serde::Deserialize)]
pub struct GuestTriggerInput {
    pub r#type: String,
    pub function_id: String,
    pub config: Value,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// Parse and validate a guest trigger registration, in the guest's own
/// vocabulary. The hosting worker renames the one field onto the Rust sdk's
/// struct; `FakeEngine` calls this directly, so the two refuse identically.
pub fn parse_guest_trigger(input: Value) -> Result<GuestTriggerInput, String> {
    serde_json::from_value(input).map_err(|e| format!("invalid trigger registration: {e}"))
}

/// A trigger-type callback as the engine describes it, in this crate's own
/// vocabulary rather than the SDK's `TriggerConfig`.
///
/// It exists because [`trigger_callback_payload`] must stay ONE function (see
/// its doc) while living in a crate that cannot name an SDK type. The hosting
/// worker converts its `TriggerConfig` into this on the way in; the field
/// names are deliberately identical so the mapping is obviously total.
pub struct TriggerCallback {
    pub id: String,
    pub function_id: String,
    pub config: Value,
    pub metadata: Option<Value>,
}

/// The wire shape a trigger-type callback's payload takes crossing from the
/// engine into a runtime's proxy handler (`op_iii_register_trigger_type`,
/// `ops.rs`) — the full callback, plus which of the two `TriggerHandler`
/// methods fired. ONE function builds it, used by both the hosting worker's
/// live `TriggerHandler` adapter and [`FakeEngine::fire_trigger_type_config`]
/// (its test double), so the two cannot drift into different shapes the way
/// `{method, config}` vs. `{method, id, function_id, config, metadata}`
/// already did once — a node-engine review caught a guest trigger-type
/// handler silently losing `id`/`function_id` because the fake it was tested
/// against never sent them.
///
/// Keeping this one function shared is why `TriggerCallback` exists rather
/// than the worker simply building the JSON itself.
pub fn trigger_callback_payload(config: TriggerCallback, method: &str) -> Value {
    serde_json::json!({
        "method": method,
        "id": config.id,
        "function_id": config.function_id,
        "config": config.config,
        "metadata": config.metadata,
    })
}

/// What each id was published with: `(id, description, handler)`.
#[cfg(test)]
type RegisteredHandlers = Arc<std::sync::Mutex<Vec<(String, Option<String>, ProxyHandler)>>>;
#[cfg(test)]
type RegisteredFormats =
    Arc<std::sync::Mutex<std::collections::HashMap<String, (Option<Value>, Option<Value>)>>>;

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
    /// `Arc` so the `'static` unregister closure can remove its own entry —
    /// a fake whose unregister only counted would let a later task assert
    /// "torn-down functions are uncallable" and pass without teardown ever
    /// removing anything.
    handlers: RegisteredHandlers,
    /// `(request_format, response_format)` per registered id — a separate
    /// map rather than widening the `handlers` tuple, for the same reason
    /// `timeouts` is separate from `calls`.
    formats: RegisteredFormats,
    unregisters: Arc<std::sync::atomic::AtomicUsize>,
    hang: std::sync::atomic::AtomicBool,
    route_back: std::sync::atomic::AtomicBool,
    delay: std::sync::Mutex<Option<std::time::Duration>>,
    /// `(id, input)` for every trigger currently registered.
    triggers: std::sync::Mutex<Vec<(String, Value)>>,
    /// Trigger-type handlers, keyed by type id. `Arc` for the same reason as
    /// `handlers`: `register_trigger_type`'s returned `'static` closure must
    /// be able to remove its own entry after this method returns.
    trigger_types: Arc<std::sync::Mutex<std::collections::HashMap<String, ProxyHandler>>>,
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

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    /// `timeout_ms` as received by `Engine::call`, one per entry in
    /// `calls()`, same order. What `op_iii_call`'s `iii.trigger` timeout
    /// clamp is verified against — the fake doesn't enforce or interpret the
    /// value, only records it.
    pub fn call_timeouts(&self) -> Vec<u64> {
        self.timeouts.lock().unwrap().clone()
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

    /// The `input` each currently-registered trigger was registered with.
    pub fn registered_triggers(&self) -> Vec<Value> {
        self.triggers
            .lock()
            .unwrap()
            .iter()
            .map(|(_, input)| input.clone())
            .collect()
    }

    pub fn registered_trigger_types(&self) -> Vec<String> {
        self.trigger_types.lock().unwrap().keys().cloned().collect()
    }

    /// Drive a registered trigger type's callback the way the engine would.
    /// `method` is `"registerTrigger"` or `"unregisterTrigger"`.
    ///
    /// Sends `{method, config}` — narrower than the live engine's
    /// `TriggerHandlerAdapter`, which also sends `id`/`function_id`/
    /// `metadata` (see `fire_trigger_type_config` for that full shape).
    /// Kept at this 3-argument signature — pinned since Task 9 — for
    /// `engine.rs`'s own `Engine` trait tests below, which only assert on
    /// `method`/`config` reaching a `ProxyHandler` and have no reason to
    /// carry the extra fields.
    pub async fn fire_trigger_type(
        &self,
        type_id: &str,
        method: &str,
        config: Value,
    ) -> Result<Value, String> {
        let handler = self.trigger_types.lock().unwrap().get(type_id).cloned();
        match handler {
            Some(h) => h(serde_json::json!({ "method": method, "config": config })).await,
            None => Err(format!("no such trigger type: {type_id}")),
        }
    }

    /// Drive a registered trigger type's callback with the FULL wire shape
    /// the live engine sends — `trigger_callback_payload`, the same builder
    /// `TriggerHandlerAdapter` uses — rather than `fire_trigger_type`'s
    /// abbreviated `{method, config}`. Use this for anything that exercises
    /// `op_iii_register_trigger_type`'s proxy handler: a real
    /// `TriggerHandler` implementation needs `id` (which trigger instance)
    /// and `function_id` (which function to invoke), neither recoverable
    /// from `config` alone, and `fire_trigger_type`'s narrower shape cannot
    /// prove either one crosses into the guest.
    pub async fn fire_trigger_type_config(
        &self,
        type_id: &str,
        method: &str,
        config: TriggerCallback,
    ) -> Result<Value, String> {
        let handler = self.trigger_types.lock().unwrap().get(type_id).cloned();
        match handler {
            Some(h) => h(trigger_callback_payload(config, method)).await,
            None => Err(format!("no such trigger type: {type_id}")),
        }
    }
}

#[cfg(test)]
impl Engine for FakeEngine {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
        _action: Option<String>,
    ) -> BoxFuture<'static, CallResult> {
        self.calls
            .lock()
            .unwrap()
            .push((fn_id.clone(), payload.clone()));
        self.timeouts.lock().unwrap().push(timeout_ms);

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
                let delay = *self.delay.lock().unwrap();
                return Box::pin(async move {
                    if let Some(d) = delay {
                        tokio::time::sleep(d).await;
                    }
                    out
                });
            }
        }

        let out = self
            .responses
            .lock()
            .unwrap()
            .get(&fn_id)
            .cloned()
            .unwrap_or_else(|| Err(format!("no such function: {fn_id}")));
        let delay = *self.delay.lock().unwrap();
        Box::pin(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            out
        })
    }

    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        request_format: Option<Value>,
        response_format: Option<Value>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        self.formats
            .lock()
            .unwrap()
            .insert(fn_id.clone(), (request_format, response_format));
        self.handlers
            .lock()
            .unwrap()
            .push((fn_id.clone(), description, handler));
        let counter = self.unregisters.clone();
        let handlers = self.handlers.clone();
        let formats = self.formats.clone();
        Box::new(move || {
            // Remove, not just count: the fake must be able to show that an
            // unregistered id is genuinely gone.
            formats.lock().unwrap().remove(&fn_id);
            handlers.lock().unwrap().retain(|(id, _, _)| *id != fn_id);
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }

    fn register_trigger(&self, input: Value) -> BoxFuture<'static, Result<String, String>> {
        // Deserialize through the SAME `guest_trigger_input` the live engine
        // uses, and refuse what it refuses. Pushing the raw `Value` was the
        // hole that hid the `type`/`trigger_type` mismatch: every test fed
        // this the node-shaped object a guest really sends, the fake stored it
        // untouched, and nothing ever discovered that `IIIEngine` could not
        // parse it. A fake that accepts what production rejects is not a
        // stand-in, it is a second implementation.
        //
        // It still STORES the guest's own object — `registered_triggers()` is
        // asserted against the shape callers wrote, which is the readable
        // thing to compare — so only the refusal is borrowed from production.
        if let Err(e) = parse_guest_trigger(input.clone()) {
            return Box::pin(async move { Err(e) });
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.triggers.lock().unwrap().push((id.clone(), input));
        Box::pin(async move { Ok(id) })
    }

    fn unregister_trigger(&self, trigger_id: String) -> BoxFuture<'static, Result<(), String>> {
        let existed = {
            let mut triggers = self.triggers.lock().unwrap();
            let before = triggers.len();
            triggers.retain(|(id, _)| *id != trigger_id);
            triggers.len() != before
        };
        Box::pin(async move {
            if existed {
                Ok(())
            } else {
                Err(format!("no such trigger: {trigger_id}"))
            }
        })
    }

    fn register_trigger_type(
        &self,
        type_id: String,
        _description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        self.trigger_types
            .lock()
            .unwrap()
            .insert(type_id.clone(), handler);
        let types = self.trigger_types.clone();
        Box::new(move || {
            types.lock().unwrap().remove(&type_id);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let un = fake.register("ns::hello".into(), None, None, None, handler);
        assert_eq!(fake.registered_ids(), vec!["ns::hello".to_string()]);
        assert_eq!(
            fake.invoke("ns::hello", json!({ "a": 1 })).await,
            Ok(json!({ "echo": { "a": 1 } }))
        );
        un();
        assert_eq!(fake.unregister_count(), 1);
        // The fake must actually forget it, so a later task cannot assert
        // "teardown makes functions uncallable" and pass spuriously.
        assert!(fake.registered_ids().is_empty());
        assert!(fake.invoke("ns::hello", json!({ "a": 1 })).await.is_err());
    }

    #[tokio::test]
    async fn the_fake_engine_records_trigger_registrations() {
        let engine = FakeEngine::new();
        let id = engine
            .register_trigger(json!({
                "type": "state",
                "function_id": "app::react",
                "config": { "key": "k" }
            }))
            .await
            .unwrap();
        assert!(!id.is_empty(), "a registered trigger must get an id");
        assert_eq!(engine.registered_triggers().len(), 1);

        engine.unregister_trigger(id.clone()).await.unwrap();
        assert!(engine.registered_triggers().is_empty());
    }

    #[tokio::test]
    async fn unregister_trigger_reports_an_unknown_id() {
        let engine = FakeEngine::new();
        let err = engine
            .unregister_trigger("no-such-trigger".to_string())
            .await
            .unwrap_err();
        assert!(err.contains("no-such-trigger"));
    }

    #[tokio::test]
    async fn fake_trigger_type_records_registrations_and_fires_the_handler() {
        let engine = FakeEngine::new();
        let handler: ProxyHandler = Arc::new(|p: Value| {
            Box::pin(async move { Ok(json!({ "saw": p })) }) as BoxFuture<'static, CallResult>
        });
        let un = engine.register_trigger_type("state".into(), None, handler);
        assert_eq!(engine.registered_trigger_types(), vec!["state".to_string()]);

        let out = engine
            .fire_trigger_type("state", "registerTrigger", json!({ "key": "k" }))
            .await
            .unwrap();
        assert_eq!(
            out,
            json!({ "saw": { "method": "registerTrigger", "config": { "key": "k" } } })
        );

        un();
        assert!(engine.registered_trigger_types().is_empty());
        assert!(engine
            .fire_trigger_type("state", "unregisterTrigger", json!({}))
            .await
            .is_err());
    }
}
