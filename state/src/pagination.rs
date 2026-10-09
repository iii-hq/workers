//! Ephemeral immutable snapshots for bounded keyed reads. No adapter writes.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::adapters::StateAdapter;
use crate::json_budget::size_within;
use crate::structs::{
    StateListEntriesInput, StateListEntriesResult, StatePrivateListEntriesInput, StateValue,
};

pub const MAX_PAGE_BYTES: usize = 8_000_000;
pub const DEFAULT_PAGE_BYTES: usize = 1_000_000;
pub const DEFAULT_PAGE_ENTRIES: usize = 100;
pub const MAX_PAGE_ENTRIES: usize = 1_000;
const MAX_SNAPSHOTS: usize = 16;
const MAX_RETAINED_BYTES: usize = 64_000_000;
const MAX_SNAPSHOT_BYTES: usize = 32_000_000;
const MAX_SNAPSHOT_ENTRIES: usize = 100_000;
const SNAPSHOT_TTL: Duration = Duration::from_secs(120);

/// Fixed, bounded error codes; neither keys nor stored values enter errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    InvalidLimits,
    UnknownCaller,
    InvalidCursor,
    SnapshotCapacity,
    RowTooLarge,
    Adapter,
    Serialization,
}
impl std::fmt::Display for PageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => {
                "INVALID_PAGE_LIMITS: limit=1..1000, max_bytes=256..8000000, scope/namespace<=1024 bytes"
            }
            Self::UnknownCaller => "UNKNOWN_CALLER: list_entries requires engine-stamped identity",
            Self::InvalidCursor => {
                "INVALID_CURSOR: invalid, expired or consumed cursor; restart without cursor"
            }
            Self::SnapshotCapacity => {
                "SNAPSHOT_CAPACITY: retention budget exhausted; retry after expiry"
            }
            Self::RowTooLarge => {
                "ROW_TOO_LARGE: a complete keyed row does not fit the page byte budget"
            }
            Self::Adapter => "LIST_ENTRIES_ERROR: snapshot capture failed",
            Self::Serialization => {
                "PAGE_SERIALIZATION_ERROR: page could not be encoded within its budget"
            }
        })
    }
}
impl std::error::Error for PageError {}

struct Snapshot {
    namespace: String,
    scope: String,
    caller: String,
    expires: Instant,
    entries: Vec<(String, StateValue)>,
    sizes: Vec<usize>,
    bytes: usize,
    offset: usize,
    limit: usize,
    max_bytes: usize,
    non_null_only: bool,
}

#[derive(Default)]
struct Cache {
    snapshots: HashMap<String, Snapshot>,
    bytes: usize,
}
impl Cache {
    fn expire(&mut self, now: Instant) {
        self.snapshots.retain(|_, snapshot| snapshot.expires > now);
        self.bytes = self.snapshots.values().map(|snapshot| snapshot.bytes).sum();
    }
}

/// Per-worker bounded cache, shared by public and every private accessor.
/// The mutex also bounds concurrent adapter captures to one. Existing adapter
/// capture has whole-scope temporary cost (KV keys + Arcs; Redis HGETALL + parse).
/// Retention charges encoded bytes plus per-row bookkeeping, not heap estimates.
#[derive(Default)]
pub struct EntryPages {
    cache: Mutex<Cache>,
}

#[derive(Serialize)]
struct PageView<'a> {
    entries: &'a [(String, StateValue)],
    next_cursor: Option<&'a str>,
    done: bool,
    offset: usize,
    total: usize,
}

impl EntryPages {
    pub async fn list(
        &self,
        adapter: &Arc<dyn StateAdapter>,
        namespace: &str,
        input: StateListEntriesInput,
    ) -> Result<StateListEntriesResult, PageError> {
        self.list_filtered(adapter, namespace, input, false).await
    }

    /// Only the private accessor exposes this opt-in; public/default reads
    /// continue to capture stored nulls and charge them against every cap.
    pub async fn list_private(
        &self,
        adapter: &Arc<dyn StateAdapter>,
        namespace: &str,
        input: StatePrivateListEntriesInput,
    ) -> Result<StateListEntriesResult, PageError> {
        self.list_filtered(adapter, namespace, input.page, input.non_null_only)
            .await
    }

    async fn list_filtered(
        &self,
        adapter: &Arc<dyn StateAdapter>,
        namespace: &str,
        input: StateListEntriesInput,
        non_null_only: bool,
    ) -> Result<StateListEntriesResult, PageError> {
        let limit = input.limit.unwrap_or(DEFAULT_PAGE_ENTRIES);
        let max_bytes = input.max_bytes.unwrap_or(DEFAULT_PAGE_BYTES);
        if !(1..=MAX_PAGE_ENTRIES).contains(&limit)
            || !(256..=MAX_PAGE_BYTES).contains(&max_bytes)
            || input.scope.len() > 1024
            || namespace.len() > 1024
        {
            return Err(PageError::InvalidLimits);
        }
        let caller = input
            .caller_worker_id
            .as_deref()
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or(PageError::UnknownCaller)?;
        let mut cache = self.cache.lock().await;
        cache.expire(Instant::now());
        if let Some(cursor) = input.cursor.as_deref() {
            // A lookup never decodes caller-controlled offsets or identity.
            if cursor.len() != 36 || Uuid::parse_str(cursor).is_err() {
                return Err(PageError::InvalidCursor);
            }
            let snapshot = cache
                .snapshots
                .get(cursor)
                .ok_or(PageError::InvalidCursor)?;
            if snapshot.namespace != namespace
                || snapshot.scope != input.scope
                || snapshot.caller != caller
                || snapshot.limit != limit
                || snapshot.max_bytes != max_bytes
                || snapshot.non_null_only != non_null_only
            {
                return Err(PageError::InvalidCursor);
            }
            let page = Self::page(snapshot)?;
            if page
                .next_cursor
                .as_ref()
                .is_some_and(|token| cache.snapshots.contains_key(token))
            {
                return Err(PageError::SnapshotCapacity);
            }
            // Rotate only after successful encoding. Failed requests cannot
            // consume or evict a live cursor belonging to somebody else.
            let mut snapshot = cache
                .snapshots
                .remove(cursor)
                .ok_or(PageError::InvalidCursor)?;
            snapshot.offset += page.entries.len();
            if let Some(next) = &page.next_cursor {
                cache.snapshots.insert(next.clone(), snapshot);
            } else {
                cache.bytes -= snapshot.bytes;
            }
            return Ok(page);
        }
        if cache.snapshots.len() >= MAX_SNAPSHOTS || cache.bytes >= MAX_RETAINED_BYTES {
            return Err(PageError::SnapshotCapacity);
        }
        let mut entries = adapter
            .list_entries(&input.scope)
            .await
            .map_err(|_| PageError::Adapter)?;
        // The adapter capture is atomic. Inspect only those captured versions,
        // before row/byte admission and metadata; never re-read or mutate history.
        if non_null_only {
            entries.retain(|(_, value)| !value.is_null());
            // Do not retain the whole-history backing allocation in a small
            // live snapshot. Whole-scope capacity is only a capture-time cost.
            entries.shrink_to_fit();
        }
        if entries.len() > MAX_SNAPSHOT_ENTRIES {
            return Err(PageError::SnapshotCapacity);
        }
        let mut sizes = Vec::with_capacity(entries.len());
        let mut bytes = 0usize;
        for row in &entries {
            let size = size_within(row, MAX_PAGE_BYTES)
                .map_err(|_| PageError::Serialization)?
                .ok_or(PageError::RowTooLarge)?;
            // Bound tiny-row bookkeeping too; encoded data is counted exactly.
            bytes = bytes
                .checked_add(size + 128)
                .ok_or(PageError::SnapshotCapacity)?;
            if bytes > MAX_SNAPSHOT_BYTES || bytes > MAX_RETAINED_BYTES - cache.bytes {
                return Err(PageError::SnapshotCapacity);
            }
            sizes.push(size);
        }
        let snapshot = Snapshot {
            namespace: namespace.to_owned(),
            scope: input.scope,
            caller: caller.to_owned(),
            expires: Instant::now() + SNAPSHOT_TTL,
            entries,
            sizes,
            bytes,
            offset: 0,
            limit,
            max_bytes,
            non_null_only,
        };
        let page = Self::page(&snapshot)?;
        if let Some(cursor) = &page.next_cursor {
            if cache.snapshots.contains_key(cursor) {
                return Err(PageError::SnapshotCapacity);
            }
            let mut snapshot = snapshot;
            snapshot.offset = page.entries.len();
            cache.bytes += snapshot.bytes;
            cache.snapshots.insert(cursor.clone(), snapshot);
        }
        Ok(page)
    }

    fn page(snapshot: &Snapshot) -> Result<StateListEntriesResult, PageError> {
        let token = Uuid::new_v4().to_string();
        let total = snapshot.entries.len();
        let base = |done| {
            size_within(
                &PageView {
                    entries: &[],
                    next_cursor: if done { None } else { Some(&token) },
                    done,
                    offset: snapshot.offset,
                    total,
                },
                snapshot.max_bytes,
            )
            .map_err(|_| PageError::Serialization)?
            .ok_or(PageError::Serialization)
        };
        let nonterminal_base = base(false)?;
        let terminal_base = base(true)?;
        // A terminal page has no token. Check the entire bounded tail first:
        // its first row may not fit with a token even when the full tail fits.
        let remaining = total - snapshot.offset;
        let terminal_rows = snapshot.sizes[snapshot.offset..]
            .iter()
            .take(snapshot.limit)
            .fold(remaining.saturating_sub(1), |bytes, size| {
                bytes.saturating_add(*size)
            });
        let terminal_fits = remaining <= snapshot.limit
            && terminal_base.saturating_add(terminal_rows) <= snapshot.max_bytes;
        let mut end = if terminal_fits {
            total
        } else {
            snapshot.offset
        };
        let mut row_bytes = 0usize;
        for size in snapshot
            .sizes
            .iter()
            .skip(snapshot.offset)
            .take(if terminal_fits { 0 } else { snapshot.limit })
        {
            let next_bytes = row_bytes + size + usize::from(end > snapshot.offset);
            let overhead = if end + 1 == total {
                terminal_base
            } else {
                nonterminal_base
            };
            if overhead + next_bytes > snapshot.max_bytes {
                break;
            }
            row_bytes = next_bytes;
            end += 1;
        }
        if end == snapshot.offset && end < total {
            return Err(PageError::RowTooLarge);
        }
        let view = PageView {
            entries: &snapshot.entries[snapshot.offset..end],
            next_cursor: if end == total { None } else { Some(&token) },
            done: end == total,
            offset: snapshot.offset,
            total,
        };
        let result = StateListEntriesResult {
            entries: view.entries.to_vec(),
            next_cursor: view.next_cursor.map(str::to_owned),
            done: view.done,
            offset: view.offset,
            total,
        };
        // Measure the FINAL response, including escaping, metadata and token.
        size_within(&result, snapshot.max_bytes)
            .map_err(|_| PageError::Serialization)?
            .ok_or(PageError::Serialization)?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "pagination_tests.rs"]
mod tests;
