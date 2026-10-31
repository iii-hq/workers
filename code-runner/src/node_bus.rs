//! The production [`Engine`] implementation for the node engine: the single
//! seam between this worker and the iii bus.
//!
//! The trait itself, its `Value`-shaped types and the `FakeEngine` test double
//! live in `iii-node-core`, which carries no SDK dependency. This file is the
//! half that owns an `IIIClient`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use iii_sdk::{
    CallAction, CallRequest, Deliver, Error, IIIClient, ObserveRejection, Observer, ObserverHandle,
    ObserverOptions, ObserverState, RegisterFunction, Source, Target,
};
use serde_json::Value;

use iii_node_core::engine::{
    hook_outcome, observer_hook_payload, parse_guest_observe, CallResult, Engine, GuestDeliver,
    GuestFunctionOptions, GuestObserveInput, GuestObserver, HookMethod, ProxyHandler, UnregisterFn,
};

/// Stands in when a caller registers without a description, on this worker's
/// own connection. Shadows the core's constant, which names node-engine —
/// the description a caller reads should name the worker they called.
const DYNAMIC_DESC: &str = "Registered at runtime by code-runner; implemented in JavaScript.";

/// The guest hooks of one function id. Read FRESH on every callback, so a
/// hook set again routes to the newest one without touching the SDK.
#[derive(Default)]
struct GuestHooks {
    added: Option<ProxyHandler>,
    removed: Option<ProxyHandler>,
}

type HookTable = Arc<Mutex<HashMap<String, GuestHooks>>>;
type ObserverTable = Arc<Mutex<HashMap<String, Observer>>>;

pub struct IIIEngine {
    iii: Arc<IIIClient>,
    /// Guest hooks by function id. A function id is in here exactly while
    /// the SDK holds this engine's two adapters for it.
    hooks: HookTable,
    /// Observers of guest functions that have hooks, by observer id: what
    /// `call_observer` calls through, since the SDK builds a call through an
    /// observer from the `Observer` itself and a guest hands back only its
    /// id. Filled by the added adapter, emptied by the removed one.
    observers: ObserverTable,
    /// Observers guests created, by id: what `dispose_observer` disposes.
    handles: Arc<Mutex<HashMap<String, ObserverHandle>>>,
}

impl IIIEngine {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            hooks: Arc::default(),
            observers: Arc::default(),
            handles: Arc::default(),
        }
    }

    fn description_or_default<'a>(&self, description: &'a Option<String>) -> &'a str {
        description.as_deref().unwrap_or(DYNAMIC_DESC)
    }

    /// Set or clear one guest hook of `fn_id`. The first hook set for an id
    /// installs BOTH SDK adapters, so this engine sees every observer of the
    /// function arrive and leave whichever hooks the guest wrote; the last
    /// one cleared removes them. A guest hook that is not set answers the
    /// way iii does by default: accept, nothing to tear down.
    fn set_hook(&self, fn_id: String, method: HookMethod, hook: Option<ProxyHandler>) {
        let mut hooks = self.hooks.lock().unwrap();
        let installed = hooks.contains_key(&fn_id);
        let entry = hooks.entry(fn_id.clone()).or_default();
        match method {
            HookMethod::ObserverAdded => entry.added = hook,
            HookMethod::ObserverRemoved => entry.removed = hook,
        }
        let empty = entry.added.is_none() && entry.removed.is_none();
        if empty {
            hooks.remove(&fn_id);
        }
        // The SDK may run a newly set hook at once for observers that are
        // already active, and the adapters lock `hooks`: release it first.
        drop(hooks);

        if empty {
            if installed {
                self.iii.clear_observer_added_for(&fn_id);
                self.iii.clear_observer_removed_for(&fn_id);
                // No removed callback will arrive for these any more.
                self.observers
                    .lock()
                    .unwrap()
                    .retain(|_, obs| obs.function_id() != fn_id);
            }
            return;
        }
        if !installed {
            self.install_adapters(&fn_id);
        }
    }

    fn install_adapters(&self, fn_id: &str) {
        let adapter = Adapter {
            fn_id: fn_id.to_string(),
            hooks: self.hooks.clone(),
            observers: self.observers.clone(),
        };
        let added = adapter.clone();
        self.iii.on_observer_added_for(fn_id, move |obs: Observer| {
            let added = added.clone();
            async move { added.on_added(obs).await }
        });
        let removed = adapter;
        self.iii
            .on_observer_removed_for(fn_id, move |obs: Observer| {
                let removed = removed.clone();
                async move { removed.on_removed(obs).await }
            });
    }

    /// Create the observer now, before the returned future is polled, so a
    /// teardown that disposes it mid-await finds its handle.
    fn observe_now(&self, input: Value) -> Result<String, String> {
        let guest = parse_guest_observe(input)?;
        crate::agent_dispatch::validate_observe(&guest.source.function_id, &guest.target)?;
        let (source, target, options) = sdk_observe(guest);
        let handle = self.iii.observe_with(source, target, options);
        // The SDK refuses a malformed observer (an invalid filter) before
        // anything is sent, and never tracks it: there is nothing to dispose.
        if handle.state() == ObserverState::Error {
            return Err(handle
                .reason()
                .map(|r| format!("{}: {}", r.code, r.message))
                .unwrap_or_else(|| "the observer was refused".to_string()));
        }
        let id = handle.id().to_string();
        self.handles.lock().unwrap().insert(id.clone(), handle);
        Ok(id)
    }
}

/// The observer as the guest's hooks see it.
fn guest_observer(obs: &Observer) -> GuestObserver {
    GuestObserver {
        id: obs.id().to_string(),
        function_id: obs.function_id().to_string(),
        args: obs.args_value().clone(),
        filters: obs.filters().clone(),
        target: obs.target().to_string(),
    }
}

/// The SDK hooks of one guest function: what they share with the engine.
#[derive(Clone)]
struct Adapter {
    fn_id: String,
    hooks: HookTable,
    observers: ObserverTable,
}

impl Adapter {
    fn guest_hook(&self, method: HookMethod) -> Option<ProxyHandler> {
        let hooks = self.hooks.lock().unwrap();
        let hooks = hooks.get(&self.fn_id)?;
        match method {
            HookMethod::ObserverAdded => hooks.added.clone(),
            HookMethod::ObserverRemoved => hooks.removed.clone(),
        }
    }

    /// Book the observer, then run the guest's `onObserverAdded`, whose
    /// refusal rejects the observer with the guest's code.
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        self.observers
            .lock()
            .unwrap()
            .insert(obs.id().to_string(), obs.clone());
        let Some(hook) = self.guest_hook(HookMethod::ObserverAdded) else {
            return Ok(());
        };
        let payload = observer_hook_payload(HookMethod::ObserverAdded, &guest_observer(&obs));
        hook_outcome(hook(payload).await).map_err(|rejection| {
            self.observers.lock().unwrap().remove(obs.id());
            ObserveRejection::new(rejection.code, rejection.message)
        })
    }

    /// Unbook the observer, then run the guest's `onObserverRemoved`. Its
    /// failure is logged; the observer is removed either way.
    async fn on_removed(&self, obs: Observer) {
        self.observers.lock().unwrap().remove(obs.id());
        let Some(hook) = self.guest_hook(HookMethod::ObserverRemoved) else {
            return;
        };
        let payload = observer_hook_payload(HookMethod::ObserverRemoved, &guest_observer(&obs));
        if let Err(error) = hook(payload).await {
            tracing::warn!(
                function_id = %self.fn_id,
                observer_id = %obs.id(),
                %error,
                "guest onObserverRemoved hook failed"
            );
        }
    }
}

/// Map the guest's observer onto the SDK's builders. Unset fields stay
/// unset, so the engine applies its own defaults.
fn sdk_observe(guest: GuestObserveInput) -> (Source, Target, ObserverOptions) {
    let mut source = Source::function(guest.source.function_id);
    if let Some(namespace) = guest.source.namespace {
        source = source.namespace(namespace);
    }
    if let Some(args) = guest.source.args {
        source = source.args(Value::Object(args));
    }
    if let Some(filters) = guest.source.filters {
        source = source.filters(Value::Object(filters));
    }
    if let Some(deliver) = guest.source.deliver {
        source = source.deliver(match deliver {
            GuestDeliver::Envelope => Deliver::Envelope,
            GuestDeliver::Input => Deliver::Input,
            GuestDeliver::Output => Deliver::Output,
        });
    }

    let mut options = ObserverOptions::default();
    let guest_options = guest.options;
    if let Some(id) = guest_options.id {
        options = options.id(id);
    }
    if let Some(metadata) = guest_options.metadata {
        options = options.metadata(metadata);
    }
    if let Some(max_fires) = guest_options.max_fires {
        options = options.max_fires(max_fires);
    }
    if let Some(expires_in_ms) = guest_options.expires_in_ms {
        options = options.expires_in(Duration::from_millis(expires_in_ms));
    }
    if let Some(max_in_flight) = guest_options.max_in_flight {
        options = options.max_in_flight(max_in_flight);
    }
    (source, Target::function(guest.target), options)
}

/// Decode the guest's action — the JSON text of its action object, such as
/// `{"type":"void"}` or `{"type":"enqueue","queue":"payments"}` — onto the
/// SDK's typed [`CallAction`].
fn parse_call_action(raw: &str) -> Result<CallAction, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid action {raw:?}: {e}"))
}

impl Engine for IIIEngine {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
        action: Option<String>,
    ) -> BoxFuture<'static, CallResult> {
        let iii = self.iii.clone();
        Box::pin(async move {
            crate::agent_dispatch::validate_call(&fn_id)?;
            let action = action.map(|raw| parse_call_action(&raw)).transpose()?;
            iii.call(CallRequest {
                function_id: fn_id,
                payload,
                action,
                timeout_ms: Some(timeout_ms),
            })
            .await
            .map_err(|e| e.to_string())
        })
    }

    fn call_observer(
        &self,
        observer_id: String,
        payload: Value,
        timeout_ms: u64,
        action: Option<String>,
    ) -> BoxFuture<'static, CallResult> {
        let observer = self.observers.lock().unwrap().get(&observer_id).cloned();
        let iii = self.iii.clone();
        Box::pin(async move {
            let observer = observer.ok_or_else(|| {
                format!("OBSERVER_NOT_FOUND: no observer `{observer_id}` of a code-runner function")
            })?;
            // The observer's creator chose its target; the guest chooses
            // what it receives. An operator-only target stays out of reach
            // here as it does for a direct call.
            crate::agent_dispatch::validate_call(observer.target())?;
            let action = action.map(|raw| parse_call_action(&raw)).transpose()?;
            let mut request = CallRequest::observer(&observer, payload)
                .timeout(Duration::from_millis(timeout_ms));
            if let Some(action) = action {
                request = request.action(action);
            }
            iii.call(request).await.map_err(|e| e.to_string())
        })
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
        let desc = self.description_or_default(&description).to_string();
        // Without an explicit format the SDK auto-extracts the schema from
        // the handler's types — `Value → Value` here, i.e. "any", which is
        // exactly what `engine::functions::info` showed before these fields
        // existed. The builder calls below are the whole feature.
        let mut registration = RegisterFunction::new_async(move |req: Value| {
            let handler = handler.clone();
            async move { handler(req).await.map_err(Error::Handler) }
        })
        .description(desc);
        if let Some(schema) = request_format {
            registration = registration.request_format(schema);
        }
        if let Some(schema) = response_format {
            registration = registration.response_format(schema);
        }
        if let Some(schema) = options.args_format {
            registration = registration.args_format_value(schema);
        }
        if options.private {
            registration = registration.private();
        }
        let function_ref = self.iii.register_function(&fn_id, registration);
        Box::new(move || function_ref.unregister())
    }

    fn on_observer_added(&self, fn_id: String, hook: Option<ProxyHandler>) {
        self.set_hook(fn_id, HookMethod::ObserverAdded, hook);
    }

    fn on_observer_removed(&self, fn_id: String, hook: Option<ProxyHandler>) {
        self.set_hook(fn_id, HookMethod::ObserverRemoved, hook);
    }

    fn observe(&self, input: Value) -> BoxFuture<'static, Result<String, String>> {
        let observed = self.observe_now(input);
        Box::pin(async move { observed })
    }

    fn dispose_observer(&self, observer_id: String) -> BoxFuture<'static, Result<(), String>> {
        let handle = self.handles.lock().unwrap().remove(&observer_id);
        Box::pin(async move {
            let handle = handle.ok_or_else(|| format!("no such observer: {observer_id}"))?;
            handle.dispose().await.map_err(|e| e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Neither constructor touches the network — `IIIClient::new` only builds
    /// local state; `connect()` (not called here) is what spawns the
    /// background thread.
    fn engine() -> IIIEngine {
        IIIEngine::new(Arc::new(IIIClient::new("ws://127.0.0.1:1")))
    }

    #[test]
    fn new_keeps_node_engines_own_fallback_description() {
        let engine = engine();
        assert_eq!(engine.description_or_default(&None), DYNAMIC_DESC);
        assert!(
            DYNAMIC_DESC.contains("code-runner"),
            "the fallback must name this worker"
        );
    }

    /// An explicit description always wins over the fallback.
    #[test]
    fn an_explicit_description_overrides_the_fallback() {
        let engine = engine();
        let explicit = Some("what this actually does".to_string());
        assert_eq!(
            engine.description_or_default(&explicit),
            "what this actually does"
        );
    }

    /// The prelude sends the guest's action object as JSON text; both
    /// actions the SDK knows decode, and anything else is a clean error.
    #[test]
    fn parse_call_action_decodes_the_guest_action_object() {
        assert!(matches!(
            parse_call_action(r#"{"type":"void"}"#).unwrap(),
            CallAction::Void
        ));
        let action = parse_call_action(r#"{"type":"enqueue","queue":"payments"}"#).unwrap();
        assert!(matches!(action, CallAction::Enqueue { queue } if queue == "payments"));
    }

    #[test]
    fn parse_call_action_reports_malformed_input_cleanly() {
        for raw in ["not json", "void", r#"{"type":"later"}"#] {
            let err = parse_call_action(raw).unwrap_err();
            assert!(
                err.contains("invalid action"),
                "unhelpful error, got: {err}"
            );
        }
    }

    /// Every field the guest may set reaches the SDK observer; the guest's
    /// id is the id the handle carries.
    #[tokio::test]
    async fn a_guest_observer_becomes_an_sdk_observer_and_can_be_disposed() {
        let engine = engine();
        let id = engine
            .observe(json!({
                "source": {
                    "function_id": "state::on-changed",
                    "filters": { "input.key": { "$eq": "k" } },
                    "deliver": "output",
                },
                "target": "app::react",
                "options": { "id": "app::watch", "max_in_flight": 1, "metadata": { "owner": "app" } },
            }))
            .await
            .unwrap();
        assert_eq!(id, "app::watch");

        engine.dispose_observer(id.clone()).await.unwrap();
        let err = engine.dispose_observer(id).await.unwrap_err();
        assert!(err.contains("no such observer"), "got {err}");
    }

    /// A filter the SDK cannot parse is refused before anything is sent,
    /// and the refusal reaches the guest instead of a pending observer.
    #[tokio::test]
    async fn an_invalid_filter_is_refused_at_observe() {
        let engine = engine();
        let err = engine
            .observe(json!({
                "source": { "function_id": "state::on-changed", "filters": { "input.key": { "$nope": 1 } } },
                "target": "app::react",
                "options": { "id": "app::bad" },
            }))
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(engine.handles.lock().unwrap().is_empty());
    }

    /// A call through an observer this engine never saw arrive fails with
    /// the engine's own code, without reaching the transport.
    #[tokio::test]
    async fn a_call_through_an_unknown_observer_is_not_found() {
        let err = engine()
            .call_observer("obs-1".into(), json!({}), 1_000, None)
            .await
            .unwrap_err();
        assert!(err.contains("OBSERVER_NOT_FOUND"), "got {err}");
    }

    /// The hook table tracks what the guest set: the first hook of an id
    /// installs it, the last one cleared removes it.
    #[test]
    fn guest_hooks_are_tracked_until_both_are_cleared() {
        let engine = engine();
        let hook: ProxyHandler = Arc::new(|_| Box::pin(async { Ok(json!(null)) }));
        engine.on_observer_added("app::x".into(), Some(hook.clone()));
        engine.on_observer_removed("app::x".into(), Some(hook));
        assert!(engine.hooks.lock().unwrap().contains_key("app::x"));

        engine.on_observer_added("app::x".into(), None);
        assert!(engine.hooks.lock().unwrap().contains_key("app::x"));
        engine.on_observer_removed("app::x".into(), None);
        assert!(engine.hooks.lock().unwrap().is_empty());

        // Clearing what was never set leaves nothing behind.
        engine.on_observer_added("app::y".into(), None);
        assert!(engine.hooks.lock().unwrap().is_empty());
    }
}
