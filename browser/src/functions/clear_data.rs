//! `browser::clear-data` — clear the session's browsing data (cookies,
//! cache, storage), the way the browser's "Clear browsing data" does. Scoped
//! to the session's own browser context; other sessions are untouched.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearDataInput {
    pub session_id: String,
    /// Clear cookies. Default true.
    #[serde(default)]
    pub cookies: Option<bool>,
    /// Clear the HTTP cache. Default true.
    #[serde(default)]
    pub cache: Option<bool>,
    /// Clear localStorage / sessionStorage / IndexedDB for the current
    /// origin. Default true.
    #[serde(default)]
    pub storage: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClearDataOutput {
    pub ok: bool,
    /// What was cleared, for the confirmation message.
    pub cleared: Vec<String>,
}
