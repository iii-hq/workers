//! The pure incremental read contract of `session::messages`.
//!
//! No IO, extraction, text filtering or cursor write. The `since` anchor is
//! already consumed: the first page is strictly after it, later pages use only
//! the server's opaque cursor. `until` is an inclusive CLIENT cut, never an API
//! field or an automatic branch choice.
//!
//! The caller correlates session, job and bank, feeds each reader its replies
//! in order and confirms persistence before using any watermark. `Complete`
//! and `AlreadyConsumed` describe only this contract: they authorize no
//! commit or model call. A classification or failure discards the whole
//! private delta; recovery needs a new decision and reader from the caller.

use std::collections::HashSet;

use serde_json::{json, Value};

pub const PAGE_SIZE: usize = 100;

/// Required resource bounds; reaching one is never a successful truncation.
/// `max_entries` counts the whole received page, past the cut too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub max_pages: usize,
    pub max_entries: usize,
}

/// The integration maps a transport error to an explicit category, never
/// by a substring of an arbitrary error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchFailure {
    InvalidCursor,
    Transient,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    InvalidInput,
    MalformedPage,
    PageTooLarge,
    RepeatedCursor,
    RepeatedEntry,
    AnchorInDelta,
    NoProgress,
    PageLimit,
    EntryLimit,
    Fetch(FetchFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationReason {
    MissingAnchor,
    MissingBoundary,
    InvalidCursor,
    BoundaryNotFound,
}

/// The envelope kept whole, content blocks uninterpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub entry_id: String,
    pub item: Value,
}

/// Only entries strictly after `since`, in reply order, up to `until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteDelta {
    pub entries: Vec<HistoryEntry>,
    /// The watermark candidate, not a write or persistence authority.
    pub watermark: String,
}

/// No incomplete variant exposes its prefix as an eligible delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    InProgress,
    Complete(CompleteDelta),
    AlreadyConsumed { watermark: String },
    NeedsClassification(ClassificationReason),
    Failed(HistoryError),
}

/// A reader of one page sequence; no transport and no fallback.
///
/// Ids and cursors are opaque. Only equality detects a repeat or backtrack;
/// a reorder of unseen ids cannot be proved locally without parent links, so
/// their order is the server's contract.
#[derive(Debug)]
pub struct DeltaReader {
    session_id: String,
    since: Option<String>,
    until: Option<String>,
    bounds: Bounds,
    pages: usize,
    received_entries: usize,
    cursor: Option<String>,
    seen_cursors: HashSet<String>,
    seen_entries: HashSet<String>,
    pending: Vec<HistoryEntry>,
    outcome: Outcome,
}

impl DeltaReader {
    pub fn new(session_id: &str, since: Option<&str>, until: Option<&str>, bounds: Bounds) -> Self {
        let outcome = if session_id.is_empty()
            || since == Some("")
            || until == Some("")
            || bounds.max_pages == 0
            || bounds.max_entries == 0
        {
            Outcome::Failed(HistoryError::InvalidInput)
        } else {
            match (since, until) {
                (_, None) => Outcome::NeedsClassification(ClassificationReason::MissingBoundary),
                (None, Some(_)) => {
                    Outcome::NeedsClassification(ClassificationReason::MissingAnchor)
                }
                (Some(anchor), Some(end)) if anchor == end => Outcome::AlreadyConsumed {
                    watermark: end.to_owned(),
                },
                _ => Outcome::InProgress,
            }
        };
        Self {
            session_id: session_id.to_owned(),
            since: since.map(str::to_owned),
            until: until.map(str::to_owned),
            bounds,
            pages: 0,
            received_entries: 0,
            cursor: None,
            seen_cursors: HashSet::new(),
            seen_entries: HashSet::new(),
            pending: Vec::new(),
            outcome,
        }
    }

    /// The next read's payload. Asking again does no IO and does not advance.
    /// The caller feeds exactly one reply per page, in order.
    pub fn next_request(&self) -> Option<Value> {
        if self.outcome != Outcome::InProgress {
            return None;
        }
        let mut request = json!({
            "session_id": self.session_id,
            "limit": PAGE_SIZE,
            "roles": ["user", "assistant"],
            "include_image_data": false,
        });
        if let Some(cursor) = &self.cursor {
            request["cursor"] = json!(cursor);
        } else if let Some(since) = &self.since {
            request["after_entry_id"] = json!(since);
        }
        Some(request)
    }

    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    /// Validates the whole page BEFORE releasing any delta: even with the cut
    /// on this page, an invalid shape, cursor or id is never a success.
    /// Replies after a terminal result are ignored; the reader stays closed.
    pub fn accept(&mut self, reply: Result<Value, FetchFailure>) {
        if self.outcome != Outcome::InProgress {
            return;
        }
        let reply = match reply {
            Ok(reply) => reply,
            Err(FetchFailure::InvalidCursor) => {
                self.stop(Outcome::NeedsClassification(
                    ClassificationReason::InvalidCursor,
                ));
                return;
            }
            Err(error) => {
                self.fail(HistoryError::Fetch(error));
                return;
            }
        };
        if let Err(error) = self.accept_page(&reply) {
            self.fail(error);
        }
    }

    fn accept_page(&mut self, reply: &Value) -> Result<(), HistoryError> {
        let object = reply.as_object().ok_or(HistoryError::MalformedPage)?;
        let messages = object
            .get("messages")
            .and_then(Value::as_array)
            .ok_or(HistoryError::MalformedPage)?;
        let next_cursor = match object.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
            _ => return Err(HistoryError::MalformedPage),
        };
        if messages.len() > PAGE_SIZE {
            return Err(HistoryError::PageTooLarge);
        }
        if let Some(next) = &next_cursor {
            if !self.seen_cursors.insert(next.clone()) {
                return Err(HistoryError::RepeatedCursor);
            }
            if messages.is_empty() {
                return Err(HistoryError::NoProgress);
            }
        }
        self.pages = self.pages.checked_add(1).ok_or(HistoryError::PageLimit)?;
        self.received_entries = self
            .received_entries
            .checked_add(messages.len())
            .ok_or(HistoryError::EntryLimit)?;
        if self.pages > self.bounds.max_pages {
            return Err(HistoryError::PageLimit);
        }
        if self.received_entries > self.bounds.max_entries {
            return Err(HistoryError::EntryLimit);
        }

        let mut page_entries = Vec::with_capacity(messages.len());
        for item in messages {
            let entry = parse_entry(item)?;
            if self.since.as_deref() == Some(entry.entry_id.as_str()) {
                return Err(HistoryError::AnchorInDelta);
            }
            if !self.seen_entries.insert(entry.entry_id.clone()) {
                return Err(HistoryError::RepeatedEntry);
            }
            page_entries.push(entry);
        }
        let cut = page_entries
            .iter()
            .position(|entry| self.until.as_deref() == Some(entry.entry_id.as_str()));
        let take = cut.map_or(page_entries.len(), |position| position + 1);
        self.pending.extend(page_entries.into_iter().take(take));
        if cut.is_some() {
            // The last kept item is exactly the cut found, never the paging
            // cursor or the leaf of a later page.
            let watermark = self
                .pending
                .last()
                .expect("corte presente")
                .entry_id
                .clone();
            let entries = std::mem::take(&mut self.pending);
            self.stop(Outcome::Complete(CompleteDelta { entries, watermark }));
        } else if let Some(next) = next_cursor {
            if self.pages >= self.bounds.max_pages {
                return Err(HistoryError::PageLimit);
            }
            if self.received_entries >= self.bounds.max_entries {
                return Err(HistoryError::EntryLimit);
            }
            self.cursor = Some(next);
        } else {
            // Absence from the delta does NOT tell an old job, a fork or a
            // wrong branch apart.
            self.stop(Outcome::NeedsClassification(
                ClassificationReason::BoundaryNotFound,
            ));
        }
        Ok(())
    }

    fn fail(&mut self, error: HistoryError) {
        self.stop(Outcome::Failed(error));
    }

    fn stop(&mut self, outcome: Outcome) {
        self.pending.clear();
        self.seen_entries.clear();
        self.seen_cursors.clear();
        self.cursor = None;
        self.outcome = outcome;
    }
}

/// Minimal validation of the requested envelope, not a content parser.
/// Extra fields and future blocks are kept; empty text is valid.
fn parse_entry(item: &Value) -> Result<HistoryEntry, HistoryError> {
    let object = item.as_object().ok_or(HistoryError::MalformedPage)?;
    let id = object
        .get("entry_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(HistoryError::MalformedPage)?;
    let message = object
        .get("message")
        .and_then(Value::as_object)
        .ok_or(HistoryError::MalformedPage)?;
    if !matches!(
        message.get("role").and_then(Value::as_str),
        Some("user" | "assistant")
    ) || !message.get("content").is_some_and(Value::is_array)
        || object.get("custom").is_some_and(|custom| !custom.is_null())
    {
        return Err(HistoryError::MalformedPage);
    }
    Ok(HistoryEntry {
        entry_id: id.to_owned(),
        item: item.clone(),
    })
}

/// Only classifies root-path evidence the caller supplies. It starts no root
/// fetch, picks no session, bank or branch, and builds no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootClassification {
    Unknown,
    Malformed,
    BoundaryMissing,
    AlreadyCovered,
    DeltaAvailable,
    FallbackRequired,
}

/// `complete` must mean the caller validated the WHOLE same active path, its
/// correlation and bounds, with no transient error; this helper cannot check
/// that. Ids compare by equality only; order is the position on the received
/// path, never lexical or time order. A partial root path never allows stale
/// or fallback, even with the anchor on it. The result may ask for a
/// fallback; no variant runs or confirms one. The caller bounds `path` too.
pub fn classify_root(
    since: Option<&str>,
    until: &str,
    path: &[&str],
    complete: bool,
) -> RootClassification {
    if until.is_empty() || since == Some("") {
        return RootClassification::Malformed;
    }
    let mut ids = HashSet::new();
    if path.iter().any(|id| id.is_empty() || !ids.insert(*id)) {
        return RootClassification::Malformed;
    }
    if !complete {
        return RootClassification::Unknown;
    }
    let Some(end) = path.iter().position(|id| *id == until) else {
        return RootClassification::BoundaryMissing;
    };
    let Some(anchor) = since.and_then(|since| path.iter().position(|id| *id == since)) else {
        return RootClassification::FallbackRequired;
    };
    if end <= anchor {
        RootClassification::AlreadyCovered
    } else {
        RootClassification::DeltaAvailable
    }
}
