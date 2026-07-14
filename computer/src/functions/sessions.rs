//! `computer::sessions::start` / `list` / `stop` — session lifecycle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::backend::Screen;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInput {
    /// computer-server endpoint to drive: a `ws://`/`wss://` url, an
    /// `http://`/`https://` url, or a bare `host:port` (ws assumed, `/ws`
    /// appended). Omit to use the configured `default_endpoint`.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Guest OS label recorded on the session and surfaced in
    /// `session-started` (`linux`, `macos`, `windows`, `android`). Omit to
    /// use the configured `os`.
    #[serde(default)]
    pub os: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StartOutput {
    /// Pass this to every other computer function.
    pub session_id: String,
    /// Normalized endpoint the session is connected to.
    pub endpoint: String,
    pub os: String,
    /// Desktop pixel dimensions; the coordinate space for `computer::act`.
    pub screen: Screen,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SessionInfo {
    pub session_id: String,
    pub endpoint: String,
    pub os: String,
    pub screen: Screen,
    pub created_ms: i64,
    pub last_used_ms: i64,
    /// True while a live screen stream is running for this session.
    pub screencast_active: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StopInput {
    /// Session to stop. Stopping an unknown or already-stopped id succeeds.
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StopOutput {
    pub ok: bool,
    /// False when the session was already gone.
    pub was_running: bool,
}
