//! `session::search` — find a chat by what was said in it.
//!
//! Fuzzy, word-based search over every session's title and the text of its
//! user and assistant messages (see [`crate::search`] for the matching and
//! ranking rules). Each hit names the session and, when the transcript
//! rather than the title answered, a snippet of the best message with the
//! matched words marked.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Deps;
use crate::error::SessionError;
use crate::search::{Range16, Snippet};
use crate::types::{SessionKind, SessionStatus};

pub const FUNCTION_ID: &str = "session::search";

/// Hits returned when `limit` is omitted.
pub const DEFAULT_LIMIT: usize = 20;
/// The most hits one call returns.
pub const MAX_LIMIT: usize = 100;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchRequest {
    /// Words to look for, in any order; each may be a word's start or carry
    /// a typo. An empty query returns no hits. At most 256 characters and
    /// 16 words; longer is `session/invalid_request`.
    pub query: String,
    /// Default 20, at most 100.
    pub limit: Option<usize>,
    /// Session kinds to search. Default `["user"]`.
    pub kinds: Option<Vec<SessionKind>>,
    /// When true, leave out sub-agent sessions (metadata `spawned_by`).
    pub roots: Option<bool>,
}

/// Where a hit's words were found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchedIn {
    /// Every word is in the title.
    Title,
    /// Some or all words are in the messages.
    Content,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchHit {
    pub session_id: String,
    pub title: String,
    pub status: SessionStatus,
    pub kind: SessionKind,
    /// Milliseconds since epoch.
    pub updated_at: i64,
    pub matched_in: MatchedIn,
    /// The matched words in `title`, UTF-16 code units `[start, end)`.
    pub title_ranges: Vec<Range16>,
    /// The best-matching message, when the messages answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<Snippet>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResponse {
    /// Best first: title hits, then a message holding the phrase, then a
    /// message holding every word, then looser hits; newest first within
    /// each.
    pub results: Vec<SearchHit>,
}

pub async fn handle(deps: &Deps, req: SearchRequest) -> Result<SearchResponse, SessionError> {
    deps.service.search(req).await
}
