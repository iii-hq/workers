//! `computer::sessions::start` / `list` / `stop` — session lifecycle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::driver::Screen;
use crate::session::Session;

/// Whether the session takes semantic (ref-based) actions.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SemanticSupport {
    pub supported: bool,
    /// Why not, when unsupported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl SemanticSupport {
    pub fn of(session: &Session) -> Self {
        if session.semantic.is_some() {
            Self {
                supported: true,
                reason: None,
            }
        } else {
            Self {
                supported: false,
                reason: Some(
                    "semantic actions need a native session on macOS or Windows; this session \
                     takes coordinates only"
                        .to_string(),
                ),
            }
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInput {
    /// OCI image (sandbox preset name or `custom_images` key) for a fresh
    /// iii-sandbox microVM desktop; overrides `endpoint`.
    // Falls back to the configured `sandbox_image`. Fixed virtual display:
    // 1:1 coordinates, no HiDPI or multi-monitor ambiguity.
    #[serde(default)]
    pub image: Option<String>,
    /// Remote desktop guest executor (`ws`/`wss`/`http`/`https` url or
    /// `host:port`). Omit this and `image` to drive the local machine.
    // Falls back to the configured `default_endpoint` before going native.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Guest OS label recorded on the session: `linux`, `macos`, `windows` or
    /// `android`. Omit to let the driver label itself.
    // Sandbox sessions are `linux`, remote ones take the configured `os`,
    // native ones this host's OS. Not an enum: the configured `os` is free text.
    #[serde(default)]
    pub os: Option<String>,
    /// Display index from `computer::displays` for a native session; omit for
    /// the display under the cursor. Ignored with `endpoint` or `image`.
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
    /// Accessibility (ref-based) actions on this session.
    pub semantic: SemanticSupport,
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
    /// Accessibility (ref-based) actions on this session.
    pub semantic: SemanticSupport,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{fake::FakeTree, SemanticSession};
    use crate::test_support::session_with;
    use std::sync::Arc;

    #[tokio::test]
    async fn semantic_support_follows_the_session() {
        let (plain, _) = session_with(None);
        let s = SemanticSupport::of(&plain);
        assert!(!s.supported);
        assert!(s.reason.unwrap().contains("coordinates only"));
        let tree = Arc::new(FakeTree::new("A", 1));
        let (native, _) = session_with(Some(SemanticSession::new(tree.factory())));
        let s = SemanticSupport::of(&native);
        assert!(s.supported && s.reason.is_none());
    }
}
