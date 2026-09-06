//! `browser::clear-data` — clear the browsing data of the site a tab is on
//! (its cookies, its storage, plus the shared cache), the way a browser's
//! per-site "Clear cookies and site data" does. `browser::clear-browser-data`
//! is the whole-profile version: every tab, every site.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearDataInput {
    pub session_id: String,
    /// Delete the cookies the current page can see (its site's cookies).
    /// Default true.
    #[serde(default)]
    pub cookies: Option<bool>,
    /// Clear the HTTP cache (shared by every tab). Default true.
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ClearBrowserDataInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClearBrowserDataOutput {
    pub ok: bool,
    /// Tabs whose page was closed to release the profile; they reopen on
    /// the next call, signed out.
    pub closed_pages: u64,
    /// The profile directory that was deleted.
    pub profile_dir: String,
}
