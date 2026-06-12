//! The 14 `session::*` functions.
//!
//! Each `<verb>.rs` holds the request/response types (serde +
//! `schemars::JsonSchema`, so the SDK emits request/response schemas)
//! and a `pub async fn handle(deps, req)` that the registration closure
//! wraps. BDD scenarios call the same `handle` functions directly, so
//! engine-free tests exercise the exact production code path
//! (handler -> service -> store -> emitter -> filters).

pub mod append;
pub mod append_many;
pub mod create;
pub mod delete;
pub mod ensure;
pub mod fork;
pub mod get;
pub mod get_message;
pub mod list;
pub mod messages;
pub mod set_active_leaf;
pub mod set_meta;
pub mod set_status;
pub mod store_protocol;
pub mod update_message;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::SessionError;
use crate::events::EventSink;
use crate::service::SessionService;

/// Everything a function handler needs. The sink is mode-dependent:
/// fs mode publishes through the local `Emitter`; bridge mode forwards
/// envelopes to the main instance (`RemotePublisher`).
pub struct Deps {
    pub service: Arc<SessionService>,
    pub sink: Arc<dyn EventSink>,
}

/// Register one typed handler under `id`, mapping `SessionError` into
/// the bus error shape (`code: message`).
fn register<Req, Resp, F, Fut>(
    iii: &Arc<III>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, SessionError>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req).await.map_err(IIIError::from) }
        })
        .description(description),
    );
}

pub fn register_all(iii: &Arc<III>, deps: &Arc<Deps>) {
    register(
        iii,
        deps,
        "session::create",
        "Create a session at status idle; fires session::created.",
        |d, r| async move { create::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::ensure",
        "Idempotently ensure a session with a given id exists; fires session::created only when it creates.",
        |d, r| async move { ensure::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::get",
        "Read one session's metadata (null when unknown).",
        |d, r| async move { get::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::list",
        "List sessions with pagination, ordering, and status/metadata filters.",
        |d, r| async move { list::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::set_meta",
        "Update a session's title/description/metadata; fires session::meta_updated.",
        |d, r| async move { set_meta::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::set_status",
        "Set status idle/working/done/error; fires session::status_changed (no-op when unchanged).",
        |d, r| async move { set_status::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::delete",
        "Delete a session and its entries; fires session::deleted.",
        |d, r| async move { delete::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::append",
        "Append one entry (idempotent on entry_id); fires session::message_added.",
        |d, r| async move { append::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::append_many",
        "Append several message entries in order; fires session::message_added per entry.",
        |d, r| async move { append_many::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::update_message",
        "Replace a message entry's content (optimistic concurrency via expected_revision); fires session::message_updated.",
        |d, r| async move { update_message::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::messages",
        "Load the active path as messages with entry ids, oldest first; pagination and role filtering.",
        |d, r| async move { messages::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::get_message",
        "Read a single entry by id (null when unknown).",
        |d, r| async move { get_message::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::fork",
        "Copy history up to an entry into a new session (copy-on-fork); fires session::created.",
        |d, r| async move { fork::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "session::set_active_leaf",
        "Move the active path to end at a given entry (branch switch).",
        |d, r| async move { set_active_leaf::handle(&d, r).await },
    );

    tracing::info!("all session::* functions registered");
}
