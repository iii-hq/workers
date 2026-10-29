//! `state::on-changed`: the change event as an observable function (iii 0.25),
//! next to the `state` trigger type.
//!
//! Its input is the event a `state` trigger handler receives. Delivery is
//! auto, with no args: an observer selects with filters on `input.scope` /
//! `input.key` (`iii.observe({ function_id: 'state::on-changed', filters },
//! target)`). After every committed change [`StateCtx`](crate::functions::StateCtx)
//! calls the function in-process, and the SDK evaluates the observers' filters
//! on that local call and reports the matches. With nobody observing, nothing
//! is built or sent ([`ChangeEmitter::is_observed`]). An engine without
//! observe (0.24) registers it as a plain function nobody can observe, so the
//! worker never calls it there; the `state` trigger type carries every change.
//!
//! Only this worker's own emission may succeed. The function is invocable by
//! any worker allowed to call it, and a forged invocation would read as a
//! state change to auto observers. Each emission runs inside an
//! [`iii_sdk::scope`] whose invocation id is a fresh single-use token, so the
//! local call's caller (`current_invocation().caller`) names that token; the
//! handler admits only a call whose caller holds a token this process issued
//! and has not consumed. Everything else fails with `FORBIDDEN`, so
//! `on: success` observers (the default) never see it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, InvocationCtx, Observe, RegisterFunction, current_invocation};
use serde_json::Value;

use crate::structs::StateEventData;

pub const ON_CHANGED_ID: &str = "state::on-changed";

pub const ON_CHANGED_DESCRIPTION: &str = "One event per committed state change (state::set, state::update, state::delete, \
     state::compare-and-set): the payload a `state` trigger handler receives, { type, event_type, \
     scope, key, old_value, new_value }. Observe it with no args and filter on input.scope / \
     input.key / input.event_type; caller.function_id is the state function that wrote, and \
     caller.parent the worker that called it. Only the state worker emits it: any other \
     invocation is refused with FORBIDDEN, so `on: success` observers (the default) see real \
     changes only. Private namespaces never emit, and `triggers_enabled: false` silences it like \
     the `state` trigger type.";

/// Bounds a routed emission (local dispatch off): the local call itself never
/// waits on the engine.
const EMIT_TIMEOUT_MS: u64 = 5_000;

/// Where change events go on their way to `state::on-changed`. Abstracts the
/// SDK so the emission gate is unit-testable without an engine.
#[async_trait::async_trait]
pub trait ChangeEmitter: Send + Sync + 'static {
    /// Whether any observer is active on `state::on-changed` (one atomic load).
    fn is_observed(&self) -> bool;

    /// Calls `state::on-changed` with `event` as its input.
    async fn emit(&self, event: Value);
}

/// The SDK-backed emitter: a local call of `state::on-changed`, gated on its
/// [`FunctionRef`].
pub struct SdkChangeEmitter {
    pub iii: Arc<IIIClient>,
    pub function: FunctionRef,
    pub emissions: Arc<Emissions>,
}

#[async_trait::async_trait]
impl ChangeEmitter for SdkChangeEmitter {
    fn is_observed(&self) -> bool {
        self.function.is_observed()
    }

    async fn emit(&self, event: Value) {
        // Read in the writing handler's own future: the emission's caller is
        // that handler (caller.function_id) and its caller (caller.parent).
        let (_issued, ctx) = self.emissions.issue(current_invocation());
        let request = TriggerRequest {
            function_id: ON_CHANGED_ID.to_string(),
            payload: event,
            action: None,
            timeout_ms: Some(EMIT_TIMEOUT_MS),
        };
        if let Err(error) = iii_sdk::scope(ctx, self.iii.trigger(request)).await {
            tracing::warn!(function_id = ON_CHANGED_ID, %error, "state change not emitted");
        }
    }
}

/// The single-use tokens of the emissions in flight.
#[derive(Debug, Default)]
pub struct Emissions {
    tokens: Mutex<HashSet<String>>,
}

/// One emission's token. Dropping it revokes the token: the handler has
/// normally consumed it already, and this covers a call that never reached
/// the handler (an error, a timeout, a dropped emission).
pub struct Issued<'a> {
    emissions: &'a Emissions,
    token: String,
}

impl Issued<'_> {
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Issued<'_> {
    fn drop(&mut self) {
        self.emissions.lock().remove(&self.token);
    }
}

impl Emissions {
    /// A fresh token, and the invocation context to emit under: `parent`
    /// (the writing handler) with the token as its invocation id, so the
    /// local call's `caller.invocation_id` carries it.
    pub fn issue(&self, parent: Option<InvocationCtx>) -> (Issued<'_>, InvocationCtx) {
        let token = uuid::Uuid::new_v4().to_string();
        self.lock().insert(token.clone());
        let ctx = match parent {
            Some(parent) => InvocationCtx {
                invocation_id: token.clone(),
                ..parent
            },
            // Outside any handler (the state::* functions always emit from
            // theirs): the emission is its own caller.
            None => InvocationCtx {
                function_id: ON_CHANGED_ID.to_string(),
                invocation_id: token.clone(),
                caller: None,
                activation: None,
                observe_depth: 0,
                metadata: None,
                traceparent: None,
                baggage: None,
            },
        };
        (
            Issued {
                emissions: self,
                token,
            },
            ctx,
        )
    }

    /// Admits the current invocation when it is one of this worker's
    /// emissions, consuming its token: an observer activation, a call from
    /// another worker or a replayed token is refused.
    pub fn admit(&self, ctx: Option<&InvocationCtx>) -> Result<(), Error> {
        let token = ctx
            .filter(|ctx| ctx.activation.is_none())
            .and_then(|ctx| ctx.caller.as_ref())
            .and_then(|caller| caller.invocation_id.as_deref());
        match token {
            Some(token) if self.lock().remove(token) => Ok(()),
            _ => Err(Error::Remote {
                code: "FORBIDDEN".to_string(),
                message: format!(
                    "{ON_CHANGED_ID} is an event source: observe it \
                     (iii.observe({{ function_id: '{ON_CHANGED_ID}', filters }}, target)); \
                     only the state worker emits its events"
                ),
                stacktrace: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.tokens.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Registers `state::on-changed`. On an engine without observe it is a plain
/// function that refuses every call.
pub fn register(iii: &IIIClient, emissions: Arc<Emissions>) -> FunctionRef {
    iii.register_function(
        ON_CHANGED_ID,
        RegisterFunction::new(move |_event: StateEventData| {
            emissions.admit(current_invocation().as_ref())
        })
        .description(ON_CHANGED_DESCRIPTION)
        .observe(Observe::auto()),
    )
}

/// Calls `state::on-changed` for one change, building the event only when
/// someone observes it and change events are enabled.
pub(crate) async fn emit_lazy<F>(emitter: &dyn ChangeEmitter, enabled: bool, build: F)
where
    F: FnOnce() -> StateEventData,
{
    if !enabled || !emitter.is_observed() {
        return;
    }
    emitter.emit(crate::events::event_json(build())).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::StateEventType;
    use iii_sdk::protocol::{Activation, Caller};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct RecordingEmitter {
        observed: AtomicBool,
        emitted: tokio::sync::Mutex<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl ChangeEmitter for RecordingEmitter {
        fn is_observed(&self) -> bool {
            self.observed.load(Ordering::SeqCst)
        }
        async fn emit(&self, event: Value) {
            self.emitted.lock().await.push(event);
        }
    }

    fn change() -> StateEventData {
        StateEventData {
            message_type: "state".to_string(),
            event_type: StateEventType::Updated,
            scope: "orders".to_string(),
            key: "o1".to_string(),
            old_value: Some(serde_json::json!({"status": "pending"})),
            new_value: serde_json::json!({"status": "shipped"}),
        }
    }

    fn writer_ctx() -> InvocationCtx {
        InvocationCtx {
            function_id: "state::set".to_string(),
            invocation_id: "inv-set".to_string(),
            caller: Some(Caller {
                worker_id: Some("writer-id".to_string()),
                worker_name: Some("writer".to_string()),
                function_id: Some("orders::place".to_string()),
                invocation_id: Some("inv-place".to_string()),
                parent: None,
            }),
            activation: None,
            observe_depth: 2,
            metadata: None,
            traceparent: None,
            baggage: None,
        }
    }

    /// What the SDK's local dispatch gives the handler: this worker as the
    /// caller, the emitting context's function and invocation ids
    /// (`dispatch.rs`, `local_ctx`).
    fn local_call(emitting: &InvocationCtx) -> InvocationCtx {
        InvocationCtx {
            function_id: ON_CHANGED_ID.to_string(),
            invocation_id: "inv-local".to_string(),
            caller: Some(Caller {
                worker_id: Some("state-id".to_string()),
                worker_name: Some("state".to_string()),
                function_id: Some(emitting.function_id.clone()),
                invocation_id: Some(emitting.invocation_id.clone()),
                parent: None,
            }),
            activation: None,
            observe_depth: emitting.observe_depth,
            metadata: None,
            traceparent: None,
            baggage: None,
        }
    }

    #[tokio::test]
    async fn unobserved_or_disabled_never_builds_or_emits() {
        let emitter = RecordingEmitter::default();
        emit_lazy(&emitter, true, || panic!("built while unobserved")).await;
        emitter.observed.store(true, Ordering::SeqCst);
        emit_lazy(&emitter, false, || panic!("built while disabled")).await;
        assert!(emitter.emitted.lock().await.is_empty());
    }

    #[tokio::test]
    async fn observed_emits_the_trigger_event_document() {
        let emitter = RecordingEmitter::default();
        emitter.observed.store(true, Ordering::SeqCst);
        emit_lazy(&emitter, true, change).await;
        let emitted = emitter.emitted.lock().await;
        assert_eq!(emitted.len(), 1);
        // The exact document a `state` trigger handler receives for it.
        assert_eq!(
            emitted[0],
            serde_json::to_value(change()).unwrap(),
            "observers and trigger handlers must see the same fields"
        );
        assert_eq!(
            emitted[0],
            serde_json::json!({
                "type": "state",
                "event_type": "state:updated",
                "scope": "orders",
                "key": "o1",
                "old_value": {"status": "pending"},
                "new_value": {"status": "shipped"},
            })
        );
    }

    #[test]
    fn the_emission_keeps_the_writer_as_its_caller() {
        let emissions = Emissions::default();
        let (issued, ctx) = emissions.issue(Some(writer_ctx()));
        assert_eq!(ctx.invocation_id, issued.token());
        assert_eq!(ctx.function_id, "state::set");
        assert_eq!(ctx.caller, writer_ctx().caller, "caller.parent of the call");
        assert_eq!(ctx.observe_depth, 2, "the loop guard keeps counting");
    }

    #[tokio::test]
    async fn the_handler_admits_its_own_emission_once() {
        let emissions = Emissions::default();
        let (_issued, emitting) = emissions.issue(Some(writer_ctx()));
        let call = local_call(&emitting);
        let admitted = iii_sdk::scope(call.clone(), async {
            emissions.admit(current_invocation().as_ref())
        })
        .await;
        assert!(admitted.is_ok());
        // Single use: replaying the token (e.g. read from an observer's
        // envelope, where it is caller.invocation_id) is refused.
        let replayed = iii_sdk::scope(call, async {
            emissions.admit(current_invocation().as_ref())
        })
        .await;
        assert!(replayed.is_err());
    }

    #[tokio::test]
    async fn forged_and_direct_invocations_are_refused() {
        let emissions = Emissions::default();
        let (_issued, emitting) = emissions.issue(Some(writer_ctx()));

        let forbidden = |result: Result<(), Error>| match result {
            Err(Error::Remote { code, .. }) => assert_eq!(code, "FORBIDDEN"),
            other => panic!("expected FORBIDDEN, got {other:?}"),
        };
        // Outside any invocation, and a remote call with no caller (an
        // engine without observe).
        forbidden(emissions.admit(None));
        let mut no_caller = local_call(&emitting);
        no_caller.caller = None;
        forbidden(emissions.admit(Some(&no_caller)));
        // Another worker's call, made from one of its handlers: the engine
        // stamps that worker as the caller, which names its own invocation.
        let mut other_worker = local_call(&writer_ctx());
        other_worker.caller = writer_ctx().caller;
        forbidden(emissions.admit(Some(&other_worker)));
        // A guessed token.
        let mut guessed = local_call(&emitting);
        guessed.caller.as_mut().unwrap().invocation_id = Some(uuid::Uuid::new_v4().to_string());
        forbidden(emissions.admit(Some(&guessed)));
        // An observer activation targeting the function, even one carrying a
        // live token.
        let mut activation = local_call(&emitting);
        activation.activation = Some(Activation {
            observer_id: "obs-1".to_string(),
            activation_id: None,
        });
        forbidden(emissions.admit(Some(&activation)));

        // None of the refusals consumed the live token.
        assert!(emissions.admit(Some(&local_call(&emitting))).is_ok());
        // A token whose emission ended is gone too.
        let (late, late_ctx) = emissions.issue(None);
        drop(late);
        forbidden(emissions.admit(Some(&local_call(&late_ctx))));
    }

    #[test]
    fn an_emission_outside_any_handler_still_gets_a_token() {
        let emissions = Emissions::default();
        let (issued, ctx) = emissions.issue(None);
        assert_eq!(ctx.invocation_id, issued.token());
        assert!(ctx.caller.is_none());
        assert!(emissions.admit(Some(&local_call(&ctx))).is_ok());
    }
}
