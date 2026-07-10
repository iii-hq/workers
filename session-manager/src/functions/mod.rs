//! The 15 `session::*` functions.
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
pub mod set_draft;
pub mod set_meta;
pub mod set_status;
pub mod store_protocol;
pub mod update_message;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

use crate::configuration::AppState;
use crate::error::SessionError;
use crate::events::EventSink;
use crate::service::SessionService;

/// Everything a function handler needs. Built fresh per call from the live
/// [`SessionRuntime`](crate::runtime::SessionRuntime), so a config reload that
/// swapped the adapter is picked up by the next call. The sink is
/// mode-dependent: fs mode publishes through the local `Emitter`; bridge mode
/// forwards envelopes to the main instance (`RemotePublisher`).
pub struct Deps {
    pub service: Arc<SessionService>,
    pub sink: Arc<dyn EventSink>,
}

/// Register one typed handler under `id`, mapping `SessionError` into
/// the bus error shape (`code: message`). Each call snapshots the live
/// runtime's `service` + `sink` from [`AppState`], so handlers never capture a
/// stale adapter across a hot-reload.
///
/// Every `session::*` function is registered with `trace_hidden: true`
/// metadata: session bookkeeping fires on every agent turn and would drown
/// trace timelines, so trace UIs hide these spans by default (users can
/// unhide them from the span filter). See
/// workers/docs/sops/trace-hidden-functions.md.
fn register<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    state: &AppState,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, SessionError>> + Send + 'static,
{
    let state = state.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let state = state.clone();
            let handler = handler.clone();
            async move {
                let deps = {
                    let rt = state.runtime.read().await;
                    Arc::new(Deps {
                        service: rt.service.clone(),
                        sink: rt.sink.clone(),
                    })
                };
                handler(deps, req).await.map_err(Error::from)
            }
        })
        .description(description)
        .metadata(json!({ "trace_hidden": true })),
    );
}

pub fn register_all(iii: &Arc<IIIClient>, state: &AppState) {
    register(
        iii,
        state,
        "session::create",
        "Create a session at status idle; fires session::created.",
        |d, r| async move { create::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::ensure",
        "Idempotently ensure a session with a given id exists; fires session::created only when it creates.",
        |d, r| async move { ensure::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::get",
        "Read one session's metadata (null when unknown).",
        |d, r| async move { get::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::list",
        "List sessions with pagination, ordering, and status/metadata filters.",
        |d, r| async move { list::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-meta",
        "Update a session's title/description/metadata; fires session::meta-updated.",
        |d, r| async move { set_meta::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-draft",
        "Park (or clear) the session's unsent composer input; event-silent, read back on SessionMeta.draft.",
        |d, r| async move { set_draft::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-status",
        "Set status idle/working/done/error; fires session::status-changed (no-op when unchanged).",
        |d, r| async move { set_status::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::delete",
        "Delete a session and its entries; fires session::deleted.",
        |d, r| async move { delete::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::append",
        "Append one entry (idempotent on entry_id); fires session::message-added.",
        |d, r| async move { append::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::append-many",
        "Append several message entries in order; fires session::message-added per entry.",
        |d, r| async move { append_many::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::update-message",
        "Replace a message entry's content (optimistic concurrency via expected_revision); fires session::message-updated.",
        |d, r| async move { update_message::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::messages",
        "Load the active path as messages with entry ids, oldest first; pagination and role filtering.",
        |d, r| async move { messages::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::get-message",
        "Read a single entry by id (null when unknown).",
        |d, r| async move { get_message::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::fork",
        "Copy history up to an entry into a new session (copy-on-fork); fires session::created.",
        |d, r| async move { fork::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-active-leaf",
        "Move the active path to end at a given entry (branch switch).",
        |d, r| async move { set_active_leaf::handle(&d, r).await },
    );

    tracing::info!("all session::* functions registered");
}
