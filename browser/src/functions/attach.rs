//! `browser::sessions::attach` and `browser::tabs::list` — drive an
//! already-running browser over CDP instead of launching a fresh one, so a
//! session reaches the user's real profile with its logins and extensions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::TabInfo;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AttachInput {
    /// CDP endpoint of the running browser: `http://127.0.0.1:9222` (resolved
    /// via `/json/version`) or a `ws://` debugger URL.
    pub cdp_url: String,
    /// Adopt the one open tab whose URL contains this substring (released
    /// untouched on stop); omit to open a fresh tab the session owns.
    #[serde(default)]
    pub adopt_url_substring: Option<String>,
    /// URL to open in the fresh tab (ignored when adopting). Omit for
    /// about:blank.
    #[serde(default)]
    pub url: Option<String>,
    /// Inspection-only session; see browser::sessions::start.
    #[serde(default)]
    pub read_only: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachOutput {
    pub session_id: String,
    pub url: String,
    pub read_only: bool,
    /// True when the session adopted an existing user tab (released, not
    /// closed, on stop); false when it opened a fresh tab it owns.
    pub adopted: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TabsListInput {
    /// CDP endpoint of the running browser, as in browser::sessions::attach.
    pub cdp_url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TabsListOutput {
    pub tabs: Vec<TabInfo>,
}
