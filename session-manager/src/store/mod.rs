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
//!   The main atomically resolves and commits appends so concurrent
//!   bridged instances share one authoritative leaf and message count.
//!
//! A future SQL/blob backend can implement the same interface.

mod bridge;
#[cfg(test)]
pub(crate) mod counting;
mod fs;

pub use bridge::BridgeStore;
pub use fs::{FsStore, decode_session_id, encode_session_id};

use async_trait::async_trait;

use crate::error::SessionError;
use crate::types::{AttachmentMeta, SessionEntry, SessionMeta};

#[derive(Debug, serde::Deserialize)]
pub struct CommitAppendResult {
    pub entry: SessionEntry,
    pub meta: SessionMeta,
    pub committed: bool,
}

/// Storage-level failure; the service maps it to `session/storage`.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StoreError(pub String);

impl From<StoreError> for SessionError {
    fn from(e: StoreError) -> Self {
        SessionError::Storage(e.0)
    }
}

/// Persistence interface for sessions.
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
    /// Atomically persist an append's entry, active leaf and updated metadata.
    /// Returns the stored entry and whether this call completed the append.
    async fn commit_append(
        &self,
        session_id: &str,
        entry: &SessionEntry,
        parent_explicit: bool,
    ) -> Result<CommitAppendResult, StoreError>;
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

    /// Every session holding `attachment_id`, in a stable order; empty when
    /// no session does. Ids are store-generated and unique per upload, but
    /// `session::fork` copies the attachments its path references under the
    /// SAME id, so one id can live (byte-identical) in several sessions.
    ///
    /// The default walks every session (`list_metas` + `list_attachments`,
    /// oldest session first): O(sessions), what a bridged instance pays per
    /// lookup. [`FsStore`] answers from an index built by one walk of its
    /// attachment folders and kept by put/delete.
    async fn attachment_owners(&self, attachment_id: &str) -> Result<Vec<String>, StoreError> {
        let mut metas = self.list_metas().await?;
        metas.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        let mut owners = Vec::new();
        for meta in metas {
            let held = self
                .list_attachments(&meta.session_id)
                .await?
                .iter()
                .any(|a| a.attachment_id == attachment_id);
            if held {
                owners.push(meta.session_id);
            }
        }
        Ok(owners)
    }
}
