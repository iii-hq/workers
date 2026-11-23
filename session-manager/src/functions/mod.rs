//! The 27 `session::*` functions: 23 session operations (among them
//! `session::read-attachment`) plus the four functions of the two chat
//! mention providers, `session::mention::*` (`@session`) and
//! `session::attachments::mention::*` (`@attachment`).
//!
//! Each `<verb>.rs` holds the request/response types (serde +
//! `schemars::JsonSchema`, so the SDK emits request/response schemas)
//! and a `pub async fn handle(deps, req)` that the registration closure
//! wraps. BDD scenarios call the same `handle` functions directly, so
//! engine-free tests exercise the exact production code path
//! (handler -> service -> store -> emitter -> filters).

pub mod append;
pub mod append_many;
pub mod attachment_mention;
pub mod create;
pub mod delete;
pub mod delete_attachment;
pub mod ensure;
pub mod fork;
pub mod get;
pub mod get_attachment;
pub mod get_message;
pub mod list;
pub mod list_attachments;
pub mod mention;
pub mod messages;
pub mod messages_range;
pub mod messages_tail;
pub mod patch_meta;
pub mod put_attachment;
pub mod read_attachment;
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
use serde::Serialize;
use serde::de::DeserializeOwned;
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
/// stale adapter across a hot-reload. `internal` hides the function from the
/// discoverable `engine::functions::list` (mutating/plumbing surface stays
/// callable by id); see harness's iii-permissions.yaml.
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
    internal: bool,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, SessionError>> + Send + 'static,
{
    let metadata = if internal {
        json!({ "internal": true, "trace_hidden": true })
    } else {
        json!({ "trace_hidden": true })
    };
    register_with_metadata(iii, state, id, description, metadata, handler);
}

/// [`register`] with explicit registration metadata (the mention provider's
/// get function carries its descriptor there).
fn register_with_metadata<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    state: &AppState,
    id: &str,
    description: &str,
    metadata: serde_json::Value,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, SessionError>> + Send + 'static,
{
    let state = state.clone();
    let reg = RegisterFunction::new_async(move |req: Req| {
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
    .metadata(metadata);
    iii.register_function(id, reg);
}

pub fn register_all(iii: &Arc<IIIClient>, state: &AppState) {
    register(
        iii,
        state,
        "session::create",
        "Create a session at status idle; calls session::on-created.",
        true,
        |d, r| async move { create::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::ensure",
        "Idempotently ensure a session with a given id exists; calls session::on-created only when it creates.",
        true,
        |d, r| async move { ensure::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::get",
        "Read one session's metadata (null when unknown).",
        false,
        |d, r| async move { get::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::list",
        "List sessions with pagination, ordering, and status/metadata filters.",
        false,
        |d, r| async move { list::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-meta",
        "Update a session's title/description/metadata; calls session::on-meta-updated.",
        true,
        |d, r| async move { set_meta::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::patch-meta",
        "Merge a session's title/description and named metadata keys (`set` replaces each key whole, `unset` removes keys, every other key is kept) under the session's write lock; calls session::on-meta-updated once with the merged metadata.",
        true,
        |d, r| async move { patch_meta::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-draft",
        "Park (or clear) the session's unsent composer input — text and attachment ids; event-silent, read back on SessionMeta.draft / draft_attachments.",
        true,
        |d, r| async move { set_draft::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-status",
        "Set status idle/working/done/error; calls session::on-status-changed (no-op when unchanged).",
        true,
        |d, r| async move { set_status::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::delete",
        "Delete a session and its entries; calls session::on-deleted.",
        true,
        |d, r| async move { delete::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::append",
        "Append one entry (idempotent on entry_id); calls session::on-message-added.",
        true,
        |d, r| async move { append::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::append-many",
        "Append several message entries in order; calls session::on-message-added per entry.",
        true,
        |d, r| async move { append_many::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::update-message",
        "Replace a message entry's content (optimistic concurrency via expected_revision); calls session::on-message-updated.",
        true,
        |d, r| async move { update_message::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::messages",
        "Load the active path as messages with entry ids, oldest first; pagination and role filtering.",
        false,
        |d, r| async move { messages::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::get-message",
        "Read a single entry by id (null when unknown).",
        false,
        |d, r| async move { get_message::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::messages-tail",
        "Newest page of the active path, walking backwards in block-aligned pages (a tool run never splits) with activity runs collapsed to placeholders; the reader a chat UI opens a session with.",
        false,
        |d, r| async move { messages_tail::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::messages-range",
        "Full entries for a span (from/to) or list (entry_ids) of the active path — how a reader fetches the calls a collapsed run left out.",
        false,
        |d, r| async move { messages_range::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::fork",
        "Copy history up to an entry into a new session (copy-on-fork); calls session::on-created.",
        true,
        |d, r| async move { fork::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::set-active-leaf",
        "Move the active path to end at a given entry (branch switch).",
        true,
        |d, r| async move { set_active_leaf::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::put-attachment",
        "Store an attachment's original bytes (base64) for a session and return the `file` content block that references it; event-silent.",
        true,
        |d, r| async move { put_attachment::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::get-attachment",
        "Read one stored attachment's metadata and bytes (null when unknown).",
        false,
        |d, r| async move { get_attachment::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::list-attachments",
        "List the metadata of every attachment stored for a session.",
        false,
        |d, r| async move { list_attachments::handle(&d, r).await },
    );
    register(
        iii,
        state,
        "session::delete-attachment",
        "Delete one stored attachment no transcript entry references (e.g. a chip removed from the composer); also drops it from the draft. Event-silent.",
        true,
        |d, r| async move { delete_attachment::handle(&d, r).await },
    );
    // The agent's way into an attached file: text in windows, a recipe for
    // everything else. The recipes are offered only for functions the engine
    // has, so the handler probes the engine through the worker's client.
    let probe = Arc::new(read_attachment::EngineProbe::new(iii.clone()));
    register(
        iii,
        state,
        read_attachment::FUNCTION_ID,
        "Read a file attached to a chat by its attachment id (a_…; session_id optional): a text file as characters in windows (offset, max_chars, next_offset), and for a PDF, office document, image or any other file `how_to_read`, the exact calls that convert it to markdown or save it into your working folder on this engine. Null when unknown.",
        false,
        move |d, r| {
            let probe = probe.clone();
            async move { read_attachment::handle(&d, probe.as_ref(), r).await }
        },
    );
    // The chat mention provider: plumbing for the console's `@session:` menu
    // and pills, so internal (agents read sessions through session::get).
    register(
        iii,
        state,
        mention::SEARCH_FN,
        "Search sessions for the chat @session mention menu, by title words or session id; best match first.",
        true,
        |d, r| async move { mention::search(&d, r).await },
    );
    register_with_metadata(
        iii,
        state,
        mention::GET_FN,
        "Resolve a @session(id=…) chat mention to its pill, preview card and agent summary; null for an unknown id. Declares the session mention provider.",
        mention::provider().metadata(),
        |d, r| async move { mention::get(&d, r).await },
    );
    // The `@attachment` provider: a chat's files, mentioned by attachment id.
    // Plumbing for the console and the apps, so internal; agents read a
    // mentioned file through session::read-attachment (the details call).
    register(
        iii,
        state,
        attachment_mention::SEARCH_FN,
        "Search the asking chat's attachments (context.session_id) for the @attachment mention menu, by name; newest first for an empty query.",
        true,
        |d, r| async move { attachment_mention::search(&d, r).await },
    );
    register_with_metadata(
        iii,
        state,
        attachment_mention::GET_FN,
        "Resolve an @attachment(id=…) chat mention to its pill, preview card and agent summary by attachment id alone; null for an unknown id. Declares the attachment mention provider.",
        attachment_mention::provider().metadata(),
        |d, r| async move { attachment_mention::get(&d, r).await },
    );

    tracing::info!("all session::* functions registered");
}
