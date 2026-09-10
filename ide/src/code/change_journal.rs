//! Bounded, in-memory snapshots for exact file diffs requested by the
//! injectable console UI. Mutation responses carry only an opaque id; full
//! before/after bodies cross the wire only when an operator opens that diff.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ChangeRecord {
    pub path: String,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Default)]
struct JournalInner {
    entries: HashMap<String, ChangeRecord>,
    order: VecDeque<String>,
    bytes: usize,
}

#[derive(Clone, Default)]
pub struct ChangeJournal {
    inner: Arc<Mutex<JournalInner>>,
}

impl ChangeJournal {
    pub fn record(&self, path: String, before: Vec<u8>, after: Vec<u8>) -> Option<String> {
        let id = Uuid::new_v4().to_string();
        let size = before.len().saturating_add(after.len());
        // Keep the journal genuinely bounded even when an operator raises the
        // normal coder write cap above this UI cache's memory budget.
        if size > MAX_BYTES {
            return None;
        }
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        while !inner.order.is_empty()
            && (inner.entries.len() >= MAX_ENTRIES || inner.bytes.saturating_add(size) > MAX_BYTES)
        {
            let Some(oldest) = inner.order.pop_front() else {
                break;
            };
            if let Some(removed) = inner.entries.remove(&oldest) {
                inner.bytes = inner
                    .bytes
                    .saturating_sub(removed.before.len().saturating_add(removed.after.len()));
            }
        }

        inner.bytes = inner.bytes.saturating_add(size);
        inner.order.push_back(id.clone());
        inner.entries.insert(
            id.clone(),
            ChangeRecord {
                path,
                before,
                after,
            },
        );
        Some(id)
    }

    pub fn get(&self, id: &str) -> Option<ChangeRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .get(id)
            .cloned()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChangeDiffInput {
    pub change_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ChangeDiffOutput {
    pub path: String,
    /// Exact pre-mutation body. Missing when either side is not UTF-8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_contents: Option<String>,
    /// Exact post-mutation body. Missing when either side is not UTF-8.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_contents: Option<String>,
    pub is_binary: bool,
}

pub fn diff(journal: &ChangeJournal, input: ChangeDiffInput) -> Result<ChangeDiffOutput, String> {
    let record = journal
        .get(&input.change_id)
        .ok_or_else(|| "change diff is no longer available; the shell worker may have restarted or evicted the snapshot".to_string())?;
    match (
        String::from_utf8(record.before),
        String::from_utf8(record.after),
    ) {
        (Ok(old_contents), Ok(new_contents)) => Ok(ChangeDiffOutput {
            path: record.path,
            old_contents: Some(old_contents),
            new_contents: Some(new_contents),
            is_binary: false,
        }),
        _ => Ok(ChangeDiffOutput {
            path: record.path,
            old_contents: None,
            new_contents: None,
            is_binary: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_exact_text_snapshots() {
        let journal = ChangeJournal::default();
        let id = journal
            .record(
                "/repo/a.txt".into(),
                b"before\n".to_vec(),
                b"after\n".to_vec(),
            )
            .unwrap();
        let out = diff(&journal, ChangeDiffInput { change_id: id }).unwrap();
        assert_eq!(out.old_contents.as_deref(), Some("before\n"));
        assert_eq!(out.new_contents.as_deref(), Some("after\n"));
        assert!(!out.is_binary);
    }

    #[test]
    fn marks_non_utf8_snapshots_binary() {
        let journal = ChangeJournal::default();
        let id = journal
            .record("/repo/a.bin".into(), vec![0xff], vec![])
            .unwrap();
        let out = diff(&journal, ChangeDiffInput { change_id: id }).unwrap();
        assert!(out.is_binary);
        assert!(out.old_contents.is_none());
    }
}
