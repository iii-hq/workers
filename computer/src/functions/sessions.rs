//! `computer::sessions::start` / `list` / `stop` — session lifecycle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::driver::Screen;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInput {
    /// Boot a fresh desktop inside an iii-sandbox microVM from this OCI image
    /// (a sandbox preset name or `custom_images` key) and drive it through iii
    /// primitives alone. A fixed virtual display means 1:1
    /// coordinates, no HiDPI or multi-monitor ambiguity. Falls back to the
    /// configured `sandbox_image` when omitted. Takes precedence over
    /// `endpoint`.
    #[serde(default)]
    pub image: Option<String>,
    /// Desktop to drive when not using a sandbox `image`. Omit (and leave
    /// `image` unset) to drive the local machine this worker runs on (native
    /// driver, nothing else to run). Pass the endpoint of a desktop guest's
    /// executor (a `ws`/`wss`/`http`/`https` url or a bare `host:port`) to
    /// drive a remote desktop; falls back to the configured
    /// `default_endpoint` when omitted.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Guest OS label recorded on the session and surfaced in
    /// `session-started` (`linux`, `macos`, `windows`, `android`). Omit to
    /// use the configured `os`.
    #[serde(default)]
    pub os: Option<String>,
    /// Display index (from `computer::displays`) for a native session. Omit to
    /// drive the display under the cursor. Ignored for a remote `endpoint`
    /// or a sandbox `image`.
    #[serde(default)]
    pub monitor: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StartOutput {
    /// Pass this to every other computer function.
    pub session_id: String,
    /// What the session drives: `native` for the local machine, or the
    /// normalized remote endpoint.
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
