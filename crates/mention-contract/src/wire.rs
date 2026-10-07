//! Request/response shapes of a provider's search and get functions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rows a search returns when the caller gives no `limit`.
pub const DEFAULT_SEARCH_LIMIT: usize = 8;
/// Upper bound on `limit`.
pub const MAX_SEARCH_LIMIT: usize = 50;

/// Input of a provider's search function.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MentionSearchRequest {
    /// What the user typed after `@<name>:`. Empty asks for the most
    /// relevant recent items.
    #[serde(default)]
    pub query: String,
    /// Maximum rows (default 8, at most 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Where the search was asked from, for providers that rank by it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<MentionSearchContext>,
}

impl MentionSearchRequest {
    /// `limit` with the default applied and clamped to `1..=MAX_SEARCH_LIMIT`.
    pub fn effective_limit(&self) -> usize {
        self.limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT)
    }

    /// The query, trimmed.
    pub fn trimmed_query(&self) -> &str {
        self.query.trim()
    }
}

/// The chat a search was asked from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionSearchContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

/// Output of a provider's search function, best match first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MentionSearchResponse {
    pub items: Vec<MentionItem>,
}

/// One search row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionItem {
    /// The id written into the token; stable for the item's lifetime.
    pub id: String,
    /// The item's name ("Fix login redirect").
    pub label: String,
    /// A short human handle shown before the label ("KAN-12").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// One line after the label ("in progress · high").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Overrides the provider's icon for this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Overrides the provider's color for this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Input of a provider's get function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionGetRequest {
    /// The id from the token.
    pub id: String,
}

/// What a chat surface renders for one mention, and what an agent reads.
/// A get function answers `null` for an id it does not know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MentionView {
    /// The canonical id (may differ from the one asked for, e.g. a key
    /// resolved to a uuid).
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Labelled values for the preview card (status, assignee, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<MentionField>,
    /// What clicking the mention opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<MentionOpen>,
    /// One line for an agent: what the item is and its state. Defaults to
    /// the label, hint, description and fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The domain object, for a worker's own preview renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// RFC 3339 time of the item's last change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

impl MentionView {
    /// The agent-facing line: `summary` when the provider wrote one,
    /// otherwise `hint label — description · field: value …`.
    pub fn agent_summary(&self) -> String {
        if let Some(summary) = self.summary.as_deref().map(str::trim) {
            if !summary.is_empty() {
                return summary.to_string();
            }
        }
        let label = serde_json::to_string(&self.label).unwrap_or_default();
        let mut line = match self.hint.as_deref() {
            Some(hint) if !hint.is_empty() => format!("{hint} {label}"),
            _ => label,
        };
        if let Some(description) = self.description.as_deref().filter(|d| !d.is_empty()) {
            line.push_str(" — ");
            line.push_str(description);
        }
        for field in &self.fields {
            line.push_str(" · ");
            line.push_str(&field.label);
            line.push_str(": ");
            line.push_str(&field.value);
        }
        line
    }
}

/// One labelled value of a preview card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionField {
    pub label: String,
    pub value: String,
    /// `neutral`, `info`, `success`, `warning` or `danger`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
}

impl MentionField {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            tone: None,
        }
    }

    pub fn tone(mut self, tone: impl Into<String>) -> Self {
        self.tone = Some(tone.into());
        self
    }
}

/// What clicking a mention opens: an injected console page, a chat
/// session, or an http(s) URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MentionOpen {
    Page {
        /// Console page id (as registered with `host.pages`).
        page: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<Value>,
    },
    Session {
        /// Session id to select.
        session: String,
    },
    Url {
        url: String,
    },
}
