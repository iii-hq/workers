//! Internal `session::store::*` protocol — the raw storage surface a
//! **main** (fs-mode) instance serves so bridged instances can use it
//! as their `SessionStore`, plus the event ingest that makes the main
//! the single fan-out point.
//!
//! Only authoritative (fs-mode) instances register these; bridge mode
//! never serves them (a bridge forwarding to itself would recurse).
//! These functions bypass all domain logic — they are deployment
//! plumbing, not an app API. Deny them to agents like every other
//! mutating surface.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::events::{Emitter, EventEnvelope};
use crate::store::SessionStore;
use crate::types::{SessionEntry, SessionMeta};

pub const GET_META: &str = "session::store::get_meta";
pub const PUT_META: &str = "session::store::put_meta";
pub const DELETE_META: &str = "session::store::delete_meta";
pub const LIST_METAS: &str = "session::store::list_metas";
pub const GET_ENTRY: &str = "session::store::get_entry";
pub const PUT_ENTRY: &str = "session::store::put_entry";
pub const LIST_ENTRIES: &str = "session::store::list_entries";
pub const DELETE_ENTRIES: &str = "session::store::delete_entries";
pub const GET_ACTIVE_LEAF: &str = "session::store::get_active_leaf";
pub const SET_ACTIVE_LEAF: &str = "session::store::set_active_leaf";
pub const DELETE_ACTIVE_LEAF: &str = "session::store::delete_active_leaf";
pub const PUBLISH_EVENTS: &str = "session::store::publish_events";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionIdRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutMetaRequest {
    pub meta: SessionMeta,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListMetasRequest {}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListMetasResponse {
    pub metas: Vec<SessionMeta>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EntryIdRequest {
    pub session_id: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutEntryRequest {
    pub session_id: String,
    pub entry: SessionEntry,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListEntriesResponse {
    pub entries: Vec<SessionEntry>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ActiveLeafResponse {
    /// `null` when the session has no active leaf.
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PublishEventsRequest {
    /// Event envelopes produced by a bridged instance's mutation.
    pub events: Vec<EventEnvelope>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PublishEventsResponse {
    /// Number of well-formed envelopes accepted and fanned out.
    pub published: usize,
}

fn storage_err(e: crate::store::StoreError) -> IIIError {
    IIIError::from(crate::error::SessionError::from(e))
}

/// Register the raw store surface backed by `store`, plus the
/// `publish_events` ingest feeding `emitter` (which fans out locally
/// and to every attached bridge).
pub fn register_store_protocol(
    iii: &Arc<III>,
    store: Arc<dyn SessionStore>,
    emitter: Arc<Emitter>,
) {
    let s = store.clone();
    iii.register_function(
        GET_META,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move { s.get_meta(&req.session_id).await.map_err(storage_err) }
        })
        .description("Internal store protocol: read one SessionMeta (null when unknown)."),
    );

    let s = store.clone();
    iii.register_function(
        PUT_META,
        RegisterFunction::new_async(move |req: PutMetaRequest| {
            let s = s.clone();
            async move {
                s.put_meta(&req.meta).await.map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: write one SessionMeta."),
    );

    let s = store.clone();
    iii.register_function(
        DELETE_META,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move {
                s.delete_meta(&req.session_id).await.map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: delete one SessionMeta."),
    );

    let s = store.clone();
    iii.register_function(
        LIST_METAS,
        RegisterFunction::new_async(move |_req: ListMetasRequest| {
            let s = s.clone();
            async move {
                let metas = s.list_metas().await.map_err(storage_err)?;
                Ok::<_, IIIError>(ListMetasResponse { metas })
            }
        })
        .description("Internal store protocol: list every SessionMeta."),
    );

    let s = store.clone();
    iii.register_function(
        GET_ENTRY,
        RegisterFunction::new_async(move |req: EntryIdRequest| {
            let s = s.clone();
            async move {
                s.get_entry(&req.session_id, &req.entry_id)
                    .await
                    .map_err(storage_err)
            }
        })
        .description("Internal store protocol: read one SessionEntry (null when unknown)."),
    );

    let s = store.clone();
    iii.register_function(
        PUT_ENTRY,
        RegisterFunction::new_async(move |req: PutEntryRequest| {
            let s = s.clone();
            async move {
                s.put_entry(&req.session_id, &req.entry)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: write one SessionEntry."),
    );

    let s = store.clone();
    iii.register_function(
        LIST_ENTRIES,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move {
                let entries = s.list_entries(&req.session_id).await.map_err(storage_err)?;
                Ok::<_, IIIError>(ListEntriesResponse { entries })
            }
        })
        .description("Internal store protocol: list every entry of a session."),
    );

    let s = store.clone();
    iii.register_function(
        DELETE_ENTRIES,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move {
                s.delete_entries(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: delete every entry of a session."),
    );

    let s = store.clone();
    iii.register_function(
        GET_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move {
                let entry_id = s
                    .get_active_leaf(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, IIIError>(ActiveLeafResponse { entry_id })
            }
        })
        .description("Internal store protocol: read a session's active leaf pointer."),
    );

    let s = store.clone();
    iii.register_function(
        SET_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: EntryIdRequest| {
            let s = s.clone();
            async move {
                s.set_active_leaf(&req.session_id, &req.entry_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: move a session's active leaf pointer."),
    );

    let s = store.clone();
    iii.register_function(
        DELETE_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let s = s.clone();
            async move {
                s.delete_active_leaf(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, IIIError>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: clear a session's active leaf pointer."),
    );

    let em = emitter.clone();
    iii.register_function(
        PUBLISH_EVENTS,
        RegisterFunction::new_async(move |req: PublishEventsRequest| {
            let em = em.clone();
            async move {
                let published = em.emit_envelopes(&req.events).await;
                Ok::<_, IIIError>(PublishEventsResponse { published })
            }
        })
        .description(
            "Internal store protocol: ingest a bridged instance's event envelopes and fan them \
             out to local subscribers and every attached bridge.",
        ),
    );

    tracing::info!("session::store::* protocol registered (12 functions)");
}
