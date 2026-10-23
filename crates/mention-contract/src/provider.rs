//! The provider descriptor a worker puts in its get function's metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::token::is_valid_name;
use crate::{CONTRACT_VERSION, METADATA_KEY, RESERVED_NAMES};

/// Declares a mention provider. Registered as `metadata.mention` on the
/// provider's **get** function — the function id that carries it is the
/// get function, so the descriptor names only the search function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionProvider {
    /// Descriptor version (currently 1).
    #[serde(default = "default_version")]
    pub v: u32,
    /// The token name: `@<name>(id="…")`. Lowercase ASCII letters, digits
    /// and `-`, starting with a letter, at most 40 characters; `fn`, `file`
    /// and `skill` are reserved.
    pub name: String,
    /// What one item is, plural, for a menu group header ("Tickets").
    pub label: String,
    /// One line on what an id refers to ("Kanban ticket, by uuid or key").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default icon name for this provider's items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Default color for this provider's items (see `color::ALL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Function id of the search function.
    pub search: String,
    /// The domain function an agent calls for the item's full details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<MentionDetails>,
}

/// Where an agent reads an item's full details: `function_id` called with
/// `{ <id_field>: "<id>" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionDetails {
    pub function_id: String,
    /// Payload field that takes the mention id. Default `id`.
    #[serde(default = "default_id_field")]
    pub id_field: String,
}

impl MentionDetails {
    /// The payload that asks `function_id` for one item.
    pub fn payload(&self, id: &str) -> Value {
        let mut payload = serde_json::Map::new();
        payload.insert(self.id_field.clone(), Value::String(id.to_string()));
        Value::Object(payload)
    }
}

/// Why a descriptor was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    InvalidName(String),
    ReservedName(String),
    EmptyField(&'static str),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::InvalidName(name) => write!(
                f,
                "mention name {name:?} must be lowercase letters, digits or '-', start with a letter, at most 40 characters"
            ),
            ProviderError::ReservedName(name) => {
                write!(f, "mention name {name:?} is reserved ({RESERVED_NAMES:?})")
            }
            ProviderError::EmptyField(field) => write!(f, "mention {field} must not be empty"),
        }
    }
}

impl std::error::Error for ProviderError {}

fn default_version() -> u32 {
    CONTRACT_VERSION
}

fn default_id_field() -> String {
    "id".to_string()
}

impl MentionProvider {
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        search: impl Into<String>,
    ) -> Self {
        Self {
            v: CONTRACT_VERSION,
            name: name.into(),
            label: label.into(),
            description: None,
            icon: None,
            color: None,
            search: search.into(),
            details: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn details(mut self, function_id: impl Into<String>, id_field: impl Into<String>) -> Self {
        self.details = Some(MentionDetails {
            function_id: function_id.into(),
            id_field: id_field.into(),
        });
        self
    }

    /// Checks the fields every consumer relies on.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if RESERVED_NAMES.contains(&self.name.as_str()) {
            return Err(ProviderError::ReservedName(self.name.clone()));
        }
        if !is_valid_name(&self.name) {
            return Err(ProviderError::InvalidName(self.name.clone()));
        }
        if self.label.trim().is_empty() {
            return Err(ProviderError::EmptyField("label"));
        }
        if self.search.trim().is_empty() {
            return Err(ProviderError::EmptyField("search"));
        }
        if let Some(details) = &self.details {
            if details.function_id.trim().is_empty() {
                return Err(ProviderError::EmptyField("details.function_id"));
            }
            if details.id_field.trim().is_empty() {
                return Err(ProviderError::EmptyField("details.id_field"));
            }
        }
        Ok(())
    }

    /// The registration metadata for the get function: the descriptor plus
    /// `internal` (agents reach items through `details`, not the mention
    /// functions) and `trace_hidden` (search runs on every keystroke).
    pub fn metadata(&self) -> Value {
        json!({
            "internal": true,
            "trace_hidden": true,
            METADATA_KEY: self,
        })
    }

    /// Reads a descriptor out of a function's metadata. `None` when the key
    /// is absent or the descriptor does not validate.
    pub fn from_metadata(metadata: &Value) -> Option<Self> {
        let raw = metadata.get(METADATA_KEY)?;
        let provider: MentionProvider = serde_json::from_value(raw.clone()).ok()?;
        provider.validate().ok()?;
        Some(provider)
    }
}
