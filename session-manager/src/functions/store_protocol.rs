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

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::configuration::AppState;
use crate::events::{Emitter, EventEnvelope};
use crate::runtime::AdapterMode;
use crate::store::SessionStore;
use crate::types::{SessionEntry, SessionMeta};

pub const GET_META: &str = "session::store::get-meta";
pub const PUT_META: &str = "session::store::put-meta";
pub const DELETE_META: &str = "session::store::delete-meta";
pub const LIST_METAS: &str = "session::store::list-metas";
pub const GET_ENTRY: &str = "session::store::get-entry";
pub const PUT_ENTRY: &str = "session::store::put-entry";
pub const LIST_ENTRIES: &str = "session::store::list-entries";
pub const DELETE_ENTRIES: &str = "session::store::delete-entries";
pub const GET_ACTIVE_LEAF: &str = "session::store::get-active-leaf";
pub const SET_ACTIVE_LEAF: &str = "session::store::set-active-leaf";
pub const DELETE_ACTIVE_LEAF: &str = "session::store::delete-active-leaf";
pub const PUBLISH_EVENTS: &str = "session::store::publish-events";

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

fn storage_err(e: crate::store::StoreError) -> Error {
    Error::from(crate::error::SessionError::from(e))
}

/// Message returned when a non-authoritative (bridge-mode) instance is asked to
/// serve the raw store protocol. Only an fs-main instance is the durable store;
/// a bridge forwarding these to itself would recurse.
const BRIDGE_MODE_REJECT: &str =
    "session::store::* is served only by an authoritative (fs-mode) instance; \
     this instance is currently in bridge mode";

/// The live store, or a rejection when the instance is not fs-main. Snapshotted
/// per call so a bridge->fs hot-reload re-enables the protocol (and fs->bridge
/// disables it) without re-registering these functions.
async fn fs_store(state: &AppState) -> Result<Arc<dyn SessionStore>, Error> {
    let rt = state.runtime.read().await;
    if rt.mode != AdapterMode::FsMain {
        return Err(Error::Handler(BRIDGE_MODE_REJECT.to_string()));
    }
    Ok(rt.store.clone())
}

/// The live local emitter, gated to fs-main like [`fs_store`].
async fn fs_emitter(state: &AppState) -> Result<Arc<Emitter>, Error> {
    let rt = state.runtime.read().await;
    if rt.mode != AdapterMode::FsMain {
        return Err(Error::Handler(BRIDGE_MODE_REJECT.to_string()));
    }
    Ok(rt.local_emitter.clone())
}

/// Register the raw store surface plus the `publish-events` ingest. Both read
/// the live runtime per call (see [`fs_store`] / [`fs_emitter`]) and reject when
/// the instance is in bridge mode, so the protocol follows adapter hot-reloads.
pub fn register_store_protocol(iii: &Arc<IIIClient>, state: AppState) {
    let st = state.clone();
    iii.register_function(
        GET_META,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store.get_meta(&req.session_id).await.map_err(storage_err)
            }
        })
        .description("Internal store protocol: read one SessionMeta (null when unknown)."),
    );

    let st = state.clone();
    iii.register_function(
        PUT_META,
        RegisterFunction::new_async(move |req: PutMetaRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store.put_meta(&req.meta).await.map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: write one SessionMeta."),
    );

    let st = state.clone();
    iii.register_function(
        DELETE_META,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .delete_meta(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: delete one SessionMeta."),
    );

    let st = state.clone();
    iii.register_function(
        LIST_METAS,
        RegisterFunction::new_async(move |_req: ListMetasRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                let metas = store.list_metas().await.map_err(storage_err)?;
                Ok::<_, Error>(ListMetasResponse { metas })
            }
        })
        .description("Internal store protocol: list every SessionMeta."),
    );

    let st = state.clone();
    iii.register_function(
        GET_ENTRY,
        RegisterFunction::new_async(move |req: EntryIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .get_entry(&req.session_id, &req.entry_id)
                    .await
                    .map_err(storage_err)
            }
        })
        .description("Internal store protocol: read one SessionEntry (null when unknown)."),
    );

    let st = state.clone();
    iii.register_function(
        PUT_ENTRY,
        RegisterFunction::new_async(move |req: PutEntryRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .put_entry(&req.session_id, &req.entry)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: write one SessionEntry."),
    );

    let st = state.clone();
    iii.register_function(
        LIST_ENTRIES,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                let entries = store
                    .list_entries(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(ListEntriesResponse { entries })
            }
        })
        .description("Internal store protocol: list every entry of a session."),
    );

    let st = state.clone();
    iii.register_function(
        DELETE_ENTRIES,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .delete_entries(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: delete every entry of a session."),
    );

    let st = state.clone();
    iii.register_function(
        GET_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                let entry_id = store
                    .get_active_leaf(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(ActiveLeafResponse { entry_id })
            }
        })
        .description("Internal store protocol: read a session's active leaf pointer."),
    );

    let st = state.clone();
    iii.register_function(
        SET_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: EntryIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .set_active_leaf(&req.session_id, &req.entry_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: move a session's active leaf pointer."),
    );

    let st = state.clone();
    iii.register_function(
        DELETE_ACTIVE_LEAF,
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let st = st.clone();
            async move {
                let store = fs_store(&st).await?;
                store
                    .delete_active_leaf(&req.session_id)
                    .await
                    .map_err(storage_err)?;
                Ok::<_, Error>(OkResponse { ok: true })
            }
        })
        .description("Internal store protocol: clear a session's active leaf pointer."),
    );

    let st = state.clone();
    iii.register_function(
        PUBLISH_EVENTS,
        RegisterFunction::new_async(move |req: PublishEventsRequest| {
            let st = st.clone();
            async move {
                let emitter = fs_emitter(&st).await?;
                let published = emitter.emit_envelopes(&req.events).await;
                Ok::<_, Error>(PublishEventsResponse { published })
            }
        })
        .description(
            "Internal store protocol: ingest a bridged instance's event envelopes and fan them \
             out to local subscribers and every attached bridge.",
        ),
    );

    tracing::info!("session::store::* protocol registered (12 functions)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BridgeBackendConfig, StorageAdapter, WorkerConfig};
    use crate::events::{BridgeSubscribers, TriggerSets};
    use crate::runtime::{build_runtime, SessionBuildContext};

    #[tokio::test]
    async fn store_protocol_gated_off_in_bridge_mode() {
        let iii = Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:59641",
            iii_sdk::InitOptions::default(),
        ));
        let cfg = WorkerConfig {
            adapter: StorageAdapter::Bridge(BridgeBackendConfig {
                url: "ws://127.0.0.1:59642".to_string(),
                timeout_ms: 5000,
            }),
            ..WorkerConfig::default()
        };
        let config_cell = Arc::new(tokio::sync::RwLock::new(Arc::new(cfg.clone())));
        let ctx = Arc::new(SessionBuildContext {
            iii,
            local_url: "ws://127.0.0.1:59641".to_string(),
            sets: TriggerSets::new(),
            bridge_subscribers: BridgeSubscribers::new(),
            config_cell,
        });
        let runtime = build_runtime(&cfg, &ctx).expect("bridge runtime builds");
        let state = AppState::new(runtime, ctx);

        assert!(
            fs_store(&state).await.is_err(),
            "raw store protocol must be refused in bridge mode"
        );
        assert!(
            fs_emitter(&state).await.is_err(),
            "publish-events ingest must be refused in bridge mode"
        );
    }
}
