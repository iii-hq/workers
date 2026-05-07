//! Per-browser subscription registry for the harness UI.
//!
//! Browsers register interest in particular sessions (or all sessions) via
//! `ui::subscribe` / `ui::unsubscribe`. The fanout keeps an in-memory map
//! of `BrowserId -> HashSet<SessionId>` (None = "all sessions, non-session
//! topics like cost/workers/approvals"). Real subscribers (agent::events,
//! state diffs, llm-budget, harness::status) are wired in later steps.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Identity of a connected browser worker. Caller-supplied; we don't mint it.
pub type BrowserId = String;

/// `None` means "subscribe to all sessions / non-session topics".
pub type Subscription = Option<String>;

#[derive(Debug, Default)]
pub struct FanoutState {
    /// browser_id -> set of subscribed session ids ("__all__" sentinel = all-sessions)
    pub subs: HashMap<BrowserId, HashSet<String>>,
}

const ALL_SESSIONS_SENTINEL: &str = "__all__";

impl FanoutState {
    pub fn subscribe(&mut self, browser: BrowserId, session: Subscription) {
        let key = session.unwrap_or_else(|| ALL_SESSIONS_SENTINEL.into());
        self.subs.entry(browser).or_default().insert(key);
    }

    pub fn unsubscribe(&mut self, browser: &str, session: Subscription) {
        let key = session.unwrap_or_else(|| ALL_SESSIONS_SENTINEL.into());
        if let Some(set) = self.subs.get_mut(browser) {
            set.remove(&key);
            if set.is_empty() {
                self.subs.remove(browser);
            }
        }
    }

    /// Browsers interested in a specific session (or in all sessions).
    pub fn subscribers_for(&self, session_id: &str) -> Vec<BrowserId> {
        self.subs
            .iter()
            .filter(|(_, set)| set.contains(session_id) || set.contains(ALL_SESSIONS_SENTINEL))
            .map(|(b, _)| b.clone())
            .collect()
    }

    /// Browsers subscribed to all-sessions topics (cost, workers, approvals).
    pub fn all_sessions_subscribers(&self) -> Vec<BrowserId> {
        self.subs
            .iter()
            .filter(|(_, set)| set.contains(ALL_SESSIONS_SENTINEL))
            .map(|(b, _)| b.clone())
            .collect()
    }

    /// Total connected browser count (any subscription).
    pub fn browser_count(&self) -> usize {
        self.subs.len()
    }
}

pub type SharedFanout = Arc<RwLock<FanoutState>>;

pub fn new_shared() -> SharedFanout {
    Arc::new(RwLock::new(FanoutState::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_inserts_and_routes_per_session() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        s.subscribe("browser-b".into(), Some("sess-2".into()));
        assert_eq!(s.subscribers_for("sess-1"), vec!["browser-a".to_string()]);
        assert_eq!(s.subscribers_for("sess-2"), vec!["browser-b".to_string()]);
        assert!(s.subscribers_for("sess-3").is_empty());
    }

    #[test]
    fn registry_routes_all_sessions_subscriber_to_any_session() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), None);
        let subs = s.subscribers_for("any-session-id");
        assert_eq!(subs, vec!["browser-a".to_string()]);
    }

    #[test]
    fn unsubscribe_evicts_browser_when_last_sub_removed() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), Some("sess-1".into()));
        s.unsubscribe("browser-a", Some("sess-1".into()));
        assert_eq!(s.browser_count(), 0);
    }

    #[test]
    fn all_sessions_subscribers_returns_only_global_subs() {
        let mut s = FanoutState::default();
        s.subscribe("browser-a".into(), None);
        s.subscribe("browser-b".into(), Some("sess-1".into()));
        let g = s.all_sessions_subscribers();
        assert_eq!(g, vec!["browser-a".to_string()]);
    }
}
