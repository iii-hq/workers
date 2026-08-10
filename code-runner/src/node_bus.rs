//! The production [`Engine`] implementation for the node engine: the single
//! seam between this worker and the iii bus.
//!
//! The trait itself, its `Value`-shaped types and the `FakeEngine` test double
//! live in `iii-node-core`, which carries no SDK dependency. This file is the
//! half that owns an `IIIClient`.

use std::sync::Arc;

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerAction, TriggerRequest};
use iii_sdk::trigger::{Trigger, TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterFunction, RegisterTriggerType};
use serde_json::Value;

use iii_node_core::engine::{
    parse_guest_trigger, trigger_callback_payload, CallResult, Engine, ProxyHandler,
    TriggerCallback, UnregisterFn,
};

/// Stands in when a caller registers without a description, on this worker's
/// own connection. Shadows the core's constant, which names node-engine —
/// the description a caller reads should name the worker they called.
const DYNAMIC_DESC: &str = "Registered at runtime by code-runner; implemented in JavaScript.";

/// Adapt the SDK's `TriggerConfig` onto the core's SDK-free mirror, so the
/// live path and the fake keep sharing one payload builder.
fn callback_of(config: TriggerConfig) -> TriggerCallback {
    TriggerCallback {
        id: config.id,
        function_id: config.function_id,
        config: config.config,
        metadata: config.metadata,
    }
}

pub struct IIIEngine {
    iii: Arc<IIIClient>,
    /// Trigger handles keyed by an id THIS engine mints, not the SDK's own.
    /// `IIIClient::register_trigger` generates its trigger id internally
    /// (see `RegisterTriggerInput`'s doc: "The `id` is auto-generated
    /// internally") and never hands it back — only an opaque `Trigger` with
    /// an `unregister()` method. `unregister_trigger` takes just an id
    /// string, decoupled from that handle, so this map is what lets a later
    /// call find the handle again.
    triggers: Arc<std::sync::Mutex<std::collections::HashMap<String, Trigger>>>,
}

impl IIIEngine {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            triggers: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn description_or_default<'a>(&self, description: &'a Option<String>) -> &'a str {
        description.as_deref().unwrap_or(DYNAMIC_DESC)
    }
}

/// Translate the guest's node-shaped registration into the Rust sdk's.
///
/// The parse and its validation live in `iii-node-core`
/// ([`parse_guest_trigger`]) so `FakeEngine` runs the exact same one — that
/// shared parse is what closed a real defect: `iii.registerTrigger` failed
/// for EVERY documented call with ``missing field `trigger_type` `` while the
/// unit tests stayed green, because the fake took a bare `Value` and never
/// deserialized at all. Only the field rename onto the SDK struct belongs
/// here, because only that names an SDK type.
fn guest_trigger_input(input: Value) -> Result<RegisterTriggerInput, String> {
    let guest = parse_guest_trigger(input)?;
    Ok(RegisterTriggerInput {
        trigger_type: guest.r#type,
        function_id: guest.function_id,
        config: guest.config,
        metadata: guest.metadata,
    })
}

/// Best-effort mapping of the guest's `action` string onto the SDK's typed
/// [`TriggerAction`]. `"void"` is the bare keyword the wire contract commits
/// to first, mirroring the enum's own `rename_all = "lowercase"` tag;
/// anything else is tried as a JSON-encoded `TriggerAction` so a future
/// queued action (`{"type":"enqueue","queue":"..."}`) round-trips without
/// touching this again. `iii.trigger`'s `action` field is what supplies the
/// `Some(_)` this now decodes; see the `parse_trigger_action_*` tests below.
fn parse_trigger_action(raw: &str) -> Result<TriggerAction, String> {
    if raw == "void" {
        return Ok(TriggerAction::Void);
    }
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
            let action = action.map(|raw| parse_trigger_action(&raw)).transpose()?;
            iii.trigger(TriggerRequest {
                function_id: fn_id,
                payload,
                action,
                timeout_ms: Some(timeout_ms),
            })
            .await
            .map_err(|e| e.to_string())
        })
    }

    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        let desc = self.description_or_default(&description).to_string();
        let function_ref = self.iii.register_function(
            &fn_id,
            RegisterFunction::new_async(move |req: Value| {
                let handler = handler.clone();
                async move { handler(req).await.map_err(Error::Handler) }
            })
            .description(desc),
        );
        Box::new(move || function_ref.unregister())
    }

    fn register_trigger(&self, input: Value) -> BoxFuture<'static, Result<String, String>> {
        let iii = self.iii.clone();
        let triggers = self.triggers.clone();
        Box::pin(async move {
            let trigger = iii
                .register_trigger(guest_trigger_input(input)?)
                .map_err(|e| e.to_string())?;
            let id = uuid::Uuid::new_v4().to_string();
            triggers.lock().unwrap().insert(id.clone(), trigger);
            Ok(id)
        })
    }

    fn unregister_trigger(&self, trigger_id: String) -> BoxFuture<'static, Result<(), String>> {
        let triggers = self.triggers.clone();
        Box::pin(async move {
            let trigger = triggers
                .lock()
                .unwrap()
                .remove(&trigger_id)
                .ok_or_else(|| format!("no such trigger: {trigger_id}"))?;
            trigger.unregister();
            Ok(())
        })
    }

    fn register_trigger_type(
        &self,
        type_id: String,
        description: Option<String>,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        let desc = self.description_or_default(&description).to_string();
        let _ = self.iii.register_trigger_type(RegisterTriggerType::new(
            type_id.clone(),
            desc,
            TriggerHandlerAdapter { handler },
        ));
        let iii = self.iii.clone();
        Box::new(move || iii.unregister_trigger_type(type_id.clone()))
    }
}
/// Adapts a Value-in/Value-out [`ProxyHandler`] to the SDK's [`TriggerHandler`]
/// trait — two typed, `async_trait`-shaped methods each returning
/// `Result<(), Error>`, rather than our one Value-to-Value shape shared with
/// `register`. The `method` field on the payload is what tells the
/// registered JS handler which of the two happened; the guest's return value
/// is otherwise discarded, since `TriggerHandler` has nowhere to put it.
///
/// Implements the trait by hand at the exact signature `#[async_trait]`
/// desugars to (a `Pin<Box<dyn Future<...> + Send>>`-returning method, which
/// is exactly what `BoxFuture` already is) rather than depending on the
/// `async-trait` macro crate itself, which `iii-sdk` does not re-export and
/// which this worker does not otherwise depend on.
struct TriggerHandlerAdapter {
    handler: ProxyHandler,
}

impl TriggerHandler for TriggerHandlerAdapter {
    fn register_trigger<'life0, 'async_trait>(
        &'life0 self,
        config: TriggerConfig,
    ) -> BoxFuture<'async_trait, Result<(), Error>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let handler = self.handler.clone();
        Box::pin(async move {
            let payload = trigger_callback_payload(callback_of(config), "registerTrigger");
            handler(payload).await.map(|_| ()).map_err(Error::Handler)
        })
    }

    fn unregister_trigger<'life0, 'async_trait>(
        &'life0 self,
        config: TriggerConfig,
    ) -> BoxFuture<'async_trait, Result<(), Error>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let handler = self.handler.clone();
        Box::pin(async move {
            let payload = trigger_callback_payload(callback_of(config), "unregisterTrigger");
            handler(payload).await.map(|_| ()).map_err(Error::Handler)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Neither constructor touches the network — `IIIClient::new` only builds
    /// local state; `connect()` (not called here) is what spawns the
    /// background thread. Same trick the real-factory shutdown test below
    /// uses for the same reason.
    #[test]
    fn new_keeps_node_engines_own_fallback_description() {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let engine = IIIEngine::new(iii);
        assert_eq!(engine.description_or_default(&None), DYNAMIC_DESC);
        assert!(
            DYNAMIC_DESC.contains("code-runner"),
            "the fallback must name this worker"
        );
    }

    /// An explicit description always wins over the fallback.
    #[test]
    fn an_explicit_description_overrides_the_fallback() {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let engine = IIIEngine::new(iii);
        let explicit = Some("what this actually does".to_string());
        assert_eq!(
            engine.description_or_default(&explicit),
            "what this actually does"
        );
    }

    /// The one field rename the guest seam exists for. Caught live by the e2e
    /// suite: `iii.registerTrigger` failed for EVERY documented call because
    /// the node-shaped object a guest writes was deserialized against the Rust
    /// sdk's `RegisterTriggerInput`, whose field is `trigger_type`.
    #[test]
    fn a_guest_registration_translates_node_type_onto_the_rust_trigger_type() {
        let input = guest_trigger_input(json!({
            "type": "app::mytype",
            "function_id": "app::react",
            "config": { "key": "k" },
            "metadata": { "note": "n" },
        }))
        .unwrap();
        assert_eq!(input.trigger_type, "app::mytype");
        assert_eq!(input.function_id, "app::react");
        assert_eq!(input.config, json!({ "key": "k" }));
        assert_eq!(input.metadata, Some(json!({ "note": "n" })));
    }

    /// `metadata` is the only optional field; everything else is required, and
    /// a missing one is reported in the GUEST's vocabulary. Naming `type` here
    /// is the whole point — the message a caller used to get named
    /// `trigger_type`, a field their sdk does not have.
    #[test]
    fn a_guest_registration_reports_a_missing_field_by_its_guest_name() {
        let err =
            guest_trigger_input(json!({ "function_id": "app::react", "config": {} })).unwrap_err();
        assert!(
            err.contains("invalid trigger registration") && err.contains("`type`"),
            "a missing type must be named as `type`, got: {err}"
        );
        assert!(
            !err.contains("trigger_type"),
            "the refusal must not name a field the guest sdk does not have: {err}"
        );
    }

    /// The Rust spelling is NOT a second way in: one field, one name. Without
    /// `type` this is a missing field, whatever else it carries.
    #[test]
    fn a_guest_registration_does_not_accept_the_rust_spelling() {
        assert!(guest_trigger_input(json!({
            "trigger_type": "app::mytype",
            "function_id": "app::react",
            "config": {},
        }))
        .is_err());
    }

    /// `parse_trigger_action` had zero coverage before Task 10 — nothing ever
    /// called `Engine::call` with `Some(_)` for `action` until `iii.trigger`
    /// did. These three cover its whole contract: the bare keyword, the
    /// JSON-decoded fallback, and the clean-error case.
    #[test]
    fn parse_trigger_action_maps_the_bare_void_keyword() {
        let action = parse_trigger_action("void").unwrap();
        assert!(matches!(action, TriggerAction::Void));
    }

    #[test]
    fn parse_trigger_action_json_decodes_anything_else() {
        let action = parse_trigger_action(r#"{"type":"enqueue","queue":"payments"}"#).unwrap();
        assert!(matches!(action, TriggerAction::Enqueue { queue } if queue == "payments"));
    }

    #[test]
    fn parse_trigger_action_reports_malformed_input_cleanly() {
        let err = parse_trigger_action("not json").unwrap_err();
        assert!(
            err.contains("invalid action"),
            "unhelpful error, got: {err}"
        );
    }
}
