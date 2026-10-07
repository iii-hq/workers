//! Worker-defined chat mentions.
//!
//! A worker becomes a **mention provider** by registering two functions:
//!
//! - a *search* function (`MentionSearchRequest` → `MentionSearchResponse`)
//!   the console calls as the user types `@<name>:<query>`, and
//! - a *get* function (`MentionGetRequest` → `Option<MentionView>`) that
//!   turns one id into what a chat surface shows (pill, preview card) and
//!   what an agent reads (a one-line summary plus the domain function that
//!   returns the full item).
//!
//! The get function carries the provider descriptor (`MentionProvider`) in
//! its registration metadata under [`METADATA_KEY`]; nothing else declares
//! it. Consumers (the console, the judge's mention hook) find providers by
//! listing functions with `include_internal: true` and reading that key.
//!
//! In text a mention is the token `@<name>(id="<id>")`, the id written as a
//! JSON string literal (see [`token`]).

mod provider;
pub mod token;
mod wire;

pub use provider::{MentionDetails, MentionProvider, ProviderError};
pub use token::{format_mention, parse_mentions, parse_prose_mentions, MentionRef};
pub use wire::{
    MentionField, MentionGetRequest, MentionItem, MentionOpen, MentionSearchContext,
    MentionSearchRequest, MentionSearchResponse, MentionView, DEFAULT_SEARCH_LIMIT,
    MAX_SEARCH_LIMIT,
};

/// The function-metadata key that holds a [`MentionProvider`].
pub const METADATA_KEY: &str = "mention";

/// Descriptor version this crate writes.
pub const CONTRACT_VERSION: u32 = 1;

/// Token names a provider may not take: the console's built-in mention
/// forms already own them (`@fn(<id>)`, `#file(<path>)`, `/skill:<id>`).
pub const RESERVED_NAMES: &[&str] = &["fn", "file", "skill"];

/// The palette a provider's `color` (and an item's) is drawn from. A chat
/// surface maps an unknown name to `neutral`.
pub mod color {
    pub const NEUTRAL: &str = "neutral";
    pub const BLUE: &str = "blue";
    pub const PURPLE: &str = "purple";
    pub const TEAL: &str = "teal";
    pub const GREEN: &str = "green";
    pub const AMBER: &str = "amber";
    pub const ROSE: &str = "rose";

    pub const ALL: &[&str] = &[NEUTRAL, BLUE, PURPLE, TEAL, GREEN, AMBER, ROSE];
}

/// The tones a `MentionField` value may carry.
pub mod tone {
    pub const NEUTRAL: &str = "neutral";
    pub const INFO: &str = "info";
    pub const SUCCESS: &str = "success";
    pub const WARNING: &str = "warning";
    pub const DANGER: &str = "danger";

    pub const ALL: &[&str] = &[NEUTRAL, INFO, SUCCESS, WARNING, DANGER];
}
