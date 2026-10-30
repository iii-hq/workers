//! Test support: a store that counts its listings.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::{CommitAppendResult, FsStore, SessionStore, StoreError};
use crate::types::{AttachmentMeta, SessionEntry, SessionMeta};

/// An [`FsStore`] that counts the listings a resync makes.
pub(crate) struct CountingStore {
    inner: FsStore,
    list_metas: AtomicUsize,
    list_entries: AtomicUsize,
}

impl CountingStore {
    pub(crate) fn new(inner: FsStore) -> Self {
        Self {
            inner,
            list_metas: AtomicUsize::new(0),
            list_entries: AtomicUsize::new(0),
        }
    }

    /// `(list_metas calls, list_entries calls)` so far.
    pub(crate) fn reads(&self) -> (usize, usize) {
        (
            self.list_metas.load(Ordering::SeqCst),
            self.list_entries.load(Ordering::SeqCst),
        )
    }
}

#[async_trait]
impl SessionStore for CountingStore {
    async fn get_meta(&self, session_id: &str) -> Result<Option<SessionMeta>, StoreError> {
        self.inner.get_meta(session_id).await
    }
    async fn put_meta(&self, meta: &SessionMeta) -> Result<(), StoreError> {
        self.inner.put_meta(meta).await
    }
    async fn delete_meta(&self, session_id: &str) -> Result<(), StoreError> {
        self.inner.delete_meta(session_id).await
    }
    async fn list_metas(&self) -> Result<Vec<SessionMeta>, StoreError> {
        self.list_metas.fetch_add(1, Ordering::SeqCst);
        self.inner.list_metas().await
    }
    async fn get_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<Option<SessionEntry>, StoreError> {
        self.inner.get_entry(session_id, entry_id).await
    }
    async fn put_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        self.inner.put_entry(session_id, entry).await
    }
    async fn commit_append(
        &self,
        session_id: &str,
        entry: &SessionEntry,
        parent_explicit: bool,
    ) -> Result<CommitAppendResult, StoreError> {
        self.inner
            .commit_append(session_id, entry, parent_explicit)
            .await
    }
    async fn list_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, StoreError> {
        self.list_entries.fetch_add(1, Ordering::SeqCst);
        self.inner.list_entries(session_id).await
    }
    async fn delete_entries(&self, session_id: &str) -> Result<(), StoreError> {
        self.inner.delete_entries(session_id).await
    }
    async fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        self.inner.get_active_leaf(session_id).await
    }
    async fn set_active_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), StoreError> {
        self.inner.set_active_leaf(session_id, entry_id).await
    }
    async fn delete_active_leaf(&self, session_id: &str) -> Result<(), StoreError> {
        self.inner.delete_active_leaf(session_id).await
    }
    async fn put_attachment(&self, meta: &AttachmentMeta, bytes: &[u8]) -> Result<(), StoreError> {
        self.inner.put_attachment(meta, bytes).await
    }
    async fn get_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<(AttachmentMeta, Vec<u8>)>, StoreError> {
        self.inner.get_attachment(session_id, attachment_id).await
    }
    async fn list_attachments(&self, session_id: &str) -> Result<Vec<AttachmentMeta>, StoreError> {
        self.inner.list_attachments(session_id).await
    }
    async fn delete_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<bool, StoreError> {
        self.inner
            .delete_attachment(session_id, attachment_id)
            .await
    }
    async fn delete_attachments(&self, session_id: &str) -> Result<(), StoreError> {
        self.inner.delete_attachments(session_id).await
    }
}
