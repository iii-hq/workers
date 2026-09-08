//! Pluggable storage backends.
//!
//! Two backends implement the same trait:
//!
//! - [`FsStore`] (default) — one append-only JSONL file per session
//!   under `data_dir` (`<encoded_session_id>.jsonl`), replayed
//!   last-wins on first access, plus one blob + metadata file per
//!   attachment under `data_dir/attachments/<encoded_session_id>/`.
//! - [`BridgeStore`] — defers every raw operation to a **main**
//!   session-manager on another iii instance via its internal
//!   `session::store::*` protocol (see `functions::store_protocol`).
//!   The bridged instance keeps all domain logic and locks; the main
//!   is pure durable storage plus the event fan-out point.
//!
//! A future SQL/blob backend can implement the same interface.

mod bridge;
mod fs;

pub use bridge::BridgeStore;
pub use fs::{decode_session_id, encode_session_id, FsStore};

use async_trait::async_trait;

use crate::error::SessionError;
use crate::types::{AttachmentMeta, SessionEntry, SessionMeta};

/// Storage-level failure; the service maps it to `session/storage`.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);

impl From<StoreError> for SessionError {
    fn from(e: StoreError) -> Self {
        SessionError::Storage(e.0)
    }
}

/// Persistence interface for sessions. Implementations only store and
/// fetch — every ordering / chain / counting rule lives in the service.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn get_meta(&self, session_id: &str) -> Result<Option<SessionMeta>, StoreError>;
    async fn put_meta(&self, meta: &SessionMeta) -> Result<(), StoreError>;
    async fn delete_meta(&self, session_id: &str) -> Result<(), StoreError>;
    /// All session metas (metas carry `session_id` inline).
    async fn list_metas(&self) -> Result<Vec<SessionMeta>, StoreError>;

    async fn get_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<Option<SessionEntry>, StoreError>;
    async fn put_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), StoreError>;
    /// All entries of a session, unordered (entries carry `id` inline).
    async fn list_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, StoreError>;
    /// Remove every entry of the session.
    async fn delete_entries(&self, session_id: &str) -> Result<(), StoreError>;

    async fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>, StoreError>;
    async fn set_active_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), StoreError>;
    async fn delete_active_leaf(&self, session_id: &str) -> Result<(), StoreError>;

    /// Store one attachment (bytes + metadata) under `meta.session_id`.
    /// Writing an `attachment_id` that already exists replaces it.
    async fn put_attachment(&self, meta: &AttachmentMeta, bytes: &[u8]) -> Result<(), StoreError>;
    /// One attachment's metadata and bytes; `None` when unknown.
    async fn get_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<(AttachmentMeta, Vec<u8>)>, StoreError>;
    /// Metadata of every attachment of a session (no bytes), unordered.
    async fn list_attachments(&self, session_id: &str) -> Result<Vec<AttachmentMeta>, StoreError>;
    /// Remove one attachment; `true` when it existed.
    async fn delete_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<bool, StoreError>;
    /// Remove every attachment of the session.
    async fn delete_attachments(&self, session_id: &str) -> Result<(), StoreError>;
}
