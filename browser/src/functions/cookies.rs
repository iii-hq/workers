//! `browser::cookies::list` / `set` / `clear` — the session's cookies, the
//! way a browser's cookie manager and "import cookies" work. Import accepts a
//! parsed list (the console reads a JSON or Netscape file into it); list reads
//! the cookies visible to the current page; clear drops them all.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One cookie as read from or written to the session. A subset of the CDP
/// cookie shape: what a person setting a cookie actually provides.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct CookieSpec {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Seconds since the Unix epoch; omitted for a session cookie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    /// `Strict`, `Lax`, or `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookiesListInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CookiesListOutput {
    pub cookies: Vec<CookieSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookiesSetInput {
    pub session_id: String,
    /// Cookies to set. A cookie without a domain is scoped to the current
    /// page's URL.
    pub cookies: Vec<CookieSpec>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CookiesSetOutput {
    pub ok: bool,
    /// How many cookies were sent.
    pub count: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CookiesClearInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CookiesClearOutput {
    pub ok: bool,
}
