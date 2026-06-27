//! The registered `harness::*` functions. Each `<verb>.rs` holds the typed
//! request/response structs (serde + `schemars::JsonSchema`) and a
//! `pub async fn handle(deps, req)` the registration closure wraps; tests call
//! the same `handle` functions directly (SOP §7).

pub mod function_resolve;
pub mod function_trigger;
pub mod on_session_deleted;
pub mod send;
pub mod spawn;
pub mod status;
pub mod stop;
pub mod subscribe;
pub mod sweep_pending;
pub mod turn;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::deps::Deps;
use crate::error::HarnessError;

pub const SEND_ID: &str = "harness::send";
pub const SEND_DESC: &str =
    "Entry point: ensure the session, persist the incoming message, and kick off a turn; returns \
     fast (or merges into a running turn).";

pub const SPAWN_ID: &str = "harness::spawn";
pub const SPAWN_DESC: &str =
    "Spawn a sub-agent in a child session; the model-facing pending trigger.";

pub const TURN_ID: &str = "harness::turn";
pub const TURN_DESC: &str =
    "Internal durable loop step (enqueued onto the default queue); not called directly.";

pub const FUNCTION_TRIGGER_ID: &str = "harness::function::trigger";
pub const FUNCTION_TRIGGER_DESC: &str =
    "Internal: invoke one iii function (unwrapped from agent_trigger), enforce the dispatch \
     policy, and capture the normalised result — or report it pending.";

pub const FUNCTION_RESOLVE_ID: &str = "harness::function::resolve";
pub const FUNCTION_RESOLVE_DESC: &str =
    "Internal: settle a pending call's result (or release a held call) and resume the parked turn.";

pub const STOP_ID: &str = "harness::stop";
pub const STOP_DESC: &str =
    "Request cancellation of an in-flight turn (cascades to spawned children).";

pub const STATUS_ID: &str = "harness::status";
pub const STATUS_DESC: &str = "Read the current turn status for a session.";

/// Register one typed handler under `id`, mapping `HarnessError` into the bus
/// error shape (`code: message`).
fn register<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, HarnessError>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req).await.map_err(Error::from) }
        })
        .description(description),
    );
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    register(iii, deps, SEND_ID, SEND_DESC, |d, r| async move {
        send::handle(&d, r).await
    });
    register(iii, deps, SPAWN_ID, SPAWN_DESC, |d, r| async move {
        spawn::handle(&d, r).await
    });
    register(iii, deps, TURN_ID, TURN_DESC, |d, r| async move {
        turn::handle(&d, r).await
    });
    register(
        iii,
        deps,
        FUNCTION_TRIGGER_ID,
        FUNCTION_TRIGGER_DESC,
        |d, r| async move { function_trigger::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        FUNCTION_RESOLVE_ID,
        FUNCTION_RESOLVE_DESC,
        |d, r| async move { function_resolve::handle(&d, r).await },
    );
    register(iii, deps, STOP_ID, STOP_DESC, |d, r| async move {
        stop::handle(&d, r).await
    });
    register(iii, deps, STATUS_ID, STATUS_DESC, |d, r| async move {
        status::handle(&d, r).await
    });

    // Ephemeral event subscriptions: the agent SUBSCRIBES / UNSUBSCRIBES by
    // calling `engine::register_trigger` / `engine::unregister_trigger`, which the
    // dispatch layer intercepts (see `subscribe::maybe_intercept`) to inject the
    // trusted session and enforce ownership — so there are no agent-facing
    // `harness::subscribe` / `harness::unsubscribe` functions.

    // Internal cron target — registered, but kept off the public catalog.
    register(
        iii,
        deps,
        sweep_pending::SWEEP_PENDING_ID,
        sweep_pending::SWEEP_PENDING_DESC,
        |d, r| async move { sweep_pending::handle(&d, r).await },
    );

    // Internal session::deleted cleanup — registered, kept off the catalog.
    register(
        iii,
        deps,
        crate::subscriptions::ON_SESSION_DELETED_ID,
        crate::subscriptions::ON_SESSION_DELETED_DESC,
        |d, r| async move { on_session_deleted::handle(&d, r).await },
    );

    // The single shared subscription fire handler — registered once, kept off
    // the catalog. Bound to by every subscription's trigger via the engine proxy.
    crate::subscriptions::notify_agent::register(deps.clone());

    tracing::info!("all harness::* functions registered");
}
