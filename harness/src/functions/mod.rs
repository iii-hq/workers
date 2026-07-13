//! The registered `harness::*` functions. Each `<verb>.rs` holds the typed
//! request/response structs (serde + `schemars::JsonSchema`) and a
//! `pub async fn handle(deps, req)` the registration closure wraps; tests call
//! the same `handle` functions directly (SOP §7).

pub mod filesystem;
pub mod function_resolve;
pub mod function_trigger;
pub mod on_session_deleted;
pub mod react;
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
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;

pub const SEND_ID: &str = "harness::send";
pub const SEND_DESC: &str =
    "Entry point: ensure the session, persist the incoming message, and kick off a turn; returns \
     fast (or merges into a running turn).";

pub const SPAWN_ID: &str = "harness::spawn";
pub const SPAWN_DESC: &str =
    "Spawn a sub-agent in a child session; the model-facing pending trigger — parks the calling \
     turn until the child resolves. Call it directly ONLY when the current turn needs the \
     child's answer; for callbacks, follow-up stages, and fan-in, register the reaction via \
     engine::register_trigger -> harness::react instead.";

pub const TURN_ID: &str = "harness::turn";
pub const TURN_DESC: &str =
    "Internal durable loop step (enqueued onto the harness-turn queue); not called directly.";

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

pub const UNQUEUE_ID: &str = "harness::unqueue";
pub const UNQUEUE_DESC: &str =
    "Internal control-plane: remove a still-parked queued message by entry_id (the console's edit-queued path).";

pub const EDIT_QUEUED_ID: &str = "harness::edit_queued";
pub const EDIT_QUEUED_DESC: &str =
    "Internal control-plane: edit a still-parked queued message in place by entry_id, preserving its queue position.";

pub const FILESYSTEM_GRANT_ID: &str = "harness::filesystem::grant";
pub const FILESYSTEM_GRANT_DESC: &str =
    "Internal control-plane: grant a session access to an additional filesystem root.";

pub const FILESYSTEM_GRANTS_ID: &str = "harness::filesystem::grants";
pub const FILESYSTEM_GRANTS_DESC: &str =
    "Internal control-plane: list additional filesystem roots granted to a session.";

pub const FILESYSTEM_REVOKE_ID: &str = "harness::filesystem::revoke";
pub const FILESYSTEM_REVOKE_DESC: &str =
    "Internal control-plane: revoke a session's access to an additional filesystem root.";

pub const FILESYSTEM_INFO_ID: &str = "harness::filesystem::info";
pub const FILESYSTEM_INFO_DESC: &str =
    "Internal control-plane: the default working-directory root new sessions are scoped to.";

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

/// Like [`register`], but tags the registration `metadata.internal = true` so
/// the default `engine::functions::list` hides it: trusted control-plane /
/// loop plumbing, invoked by id, never meant for agent discovery (mirrors the
/// deny rules in iii-permissions.yaml).
fn register_internal<Req, Resp, F, Fut>(
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
        .description(description)
        .metadata(serde_json::json!({ "internal": true })),
    );
}

/// Like [`register`], but the handler also receives the per-invocation
/// `metadata` sidecar (`engine::register_trigger`'s `metadata`). Used by the
/// trigger-bridge target `harness::react`.
fn register_with_metadata<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req, Option<Value>) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, HarnessError>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req, meta: Option<Value>| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req, meta).await.map_err(Error::from) }
        })
        .description(description),
    );
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    register_internal(iii, deps, SEND_ID, SEND_DESC, |d, r| async move {
        send::handle(&d, r).await
    });
    register(iii, deps, SPAWN_ID, SPAWN_DESC, |d, r| async move {
        spawn::handle(&d, r).await
    });
    register_internal(iii, deps, TURN_ID, TURN_DESC, |d, r| async move {
        turn::handle(&d, r).await
    });
    register_internal(
        iii,
        deps,
        FUNCTION_TRIGGER_ID,
        FUNCTION_TRIGGER_DESC,
        |d, r| async move { function_trigger::handle(&d, r).await },
    );
    register_internal(
        iii,
        deps,
        FUNCTION_RESOLVE_ID,
        FUNCTION_RESOLVE_DESC,
        |d, r| async move { function_resolve::handle(&d, r).await },
    );
    register_internal(iii, deps, STOP_ID, STOP_DESC, |d, r| async move {
        stop::handle(&d, r).await
    });
    register(iii, deps, STATUS_ID, STATUS_DESC, |d, r| async move {
        status::handle(&d, r).await
    });

    // Trusted control-plane (console) — registered, kept off the agent catalog.
    register_internal(iii, deps, UNQUEUE_ID, UNQUEUE_DESC, |d, r| async move {
        send::unqueue(&d, r).await
    });
    register_internal(
        iii,
        deps,
        EDIT_QUEUED_ID,
        EDIT_QUEUED_DESC,
        |d, r| async move { send::edit_queued(&d, r).await },
    );

    // Internal filesystem grant controls — registered for trusted callers, kept
    // off the model-facing catalog.
    register_internal(
        iii,
        deps,
        FILESYSTEM_GRANT_ID,
        FILESYSTEM_GRANT_DESC,
        |d, r| async move { filesystem::grant(&d, r).await },
    );
    register_internal(
        iii,
        deps,
        FILESYSTEM_GRANTS_ID,
        FILESYSTEM_GRANTS_DESC,
        |d, r| async move { filesystem::grants(&d, r).await },
    );
    register_internal(
        iii,
        deps,
        FILESYSTEM_REVOKE_ID,
        FILESYSTEM_REVOKE_DESC,
        |d, r| async move { filesystem::revoke(&d, r).await },
    );
    register_internal(
        iii,
        deps,
        FILESYSTEM_INFO_ID,
        FILESYSTEM_INFO_DESC,
        |d, r| async move { filesystem::info(&d, r).await },
    );

    // Internal cron target — registered, but kept off the public catalog.
    register_internal(
        iii,
        deps,
        sweep_pending::SWEEP_PENDING_ID,
        sweep_pending::SWEEP_PENDING_DESC,
        |d, r| async move { sweep_pending::handle(&d, r).await },
    );

    // Internal session::deleted cleanup — registered, kept off the catalog.
    register_internal(
        iii,
        deps,
        crate::subscriptions::ON_SESSION_DELETED_ID,
        crate::subscriptions::ON_SESSION_DELETED_DESC,
        |d, r| async move { on_session_deleted::handle(&d, r).await },
    );

    // The single shared subscription fire handler — registered once, kept off
    // the catalog. Bound to by every subscription's trigger via the engine proxy.
    crate::subscriptions::notify_agent::register(deps.clone());

    // Internal trigger-bridge target — fired only by subscriptions the agent
    // binds via engine::register_trigger. Visible in the catalog (its
    // description points binders at engine::register_trigger), but a direct
    // call arrives without the trigger metadata sidecar and no-ops; deployment
    // permission policies additionally deny it to agents. The event is the
    // payload; the reaction spec arrives as the metadata sidecar.
    register_with_metadata(
        iii,
        deps,
        react::REACT_ID,
        react::REACT_DESC,
        |d, ev: react::ReactEvent, meta| async move { react::handle(&d, ev.0, meta).await },
    );

    tracing::info!("all harness::* functions registered");
}
