//! `browser::history::list` — the session's visited pages, newest first, for
//! the history panel and address-bar suggestions. Distinct from
//! `browser::history` (back / forward / reload), which moves the page.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::HistoryVisit;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryListInput {
    pub session_id: String,
    /// Only entries whose url or title contains this (case-insensitive).
    #[serde(default)]
    pub query: Option<String>,
    /// Cap on returned entries. Default 100.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryListOutput {
    pub visits: Vec<HistoryVisit>,
}

/// Newest first, filtered and capped.
pub fn select(visits: &[HistoryVisit], query: Option<&str>, limit: usize) -> Vec<HistoryVisit> {
    let needle = query.map(|q| q.to_lowercase());
    visits
        .iter()
        .rev()
        .filter(|v| match &needle {
            Some(n) if !n.is_empty() => {
                v.url.to_lowercase().contains(n) || v.title.to_lowercase().contains(n)
            }
            _ => true,
        })
        .take(limit)
        .cloned()
        .collect()
}
