//! `computer::sessions::start` / `list` / `stop` — session lifecycle.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::driver::Screen;
use crate::session::Session;

/// Whether the session takes one kind of action, and why not.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Support {
    pub supported: bool,
    /// Why not, when unsupported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Support {
    fn yes() -> Self {
        Self {
            supported: true,
            reason: None,
        }
    }

    fn no(reason: impl Into<String>) -> Self {
        Self {
            supported: false,
            reason: Some(reason.into()),
        }
    }

    /// Semantic (ref-based) actions: native sessions only.
    pub fn semantic(session: &Session) -> Self {
        if session.semantic.is_some() {
            Self::yes()
        } else {
            Self::no(
                "semantic actions need a native session (macOS, Windows or Linux); this session \
                 takes coordinates only",
            )
        }
    }

    /// Screenshots and coordinate actions: every session but a semantic-only
    /// native one (a Wayland desktop).
    pub fn coordinates(session: &Session) -> Self {
        match session.driver().coordinates() {
            Some(reason) => Self::no(reason),
            None => Self::yes(),
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
    pub semantic: Support,
    /// Screenshots and coordinate actions (pointer, keyboard) on this session.
    pub coordinates: Support,
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
    pub semantic: Support,
    /// Screenshots and coordinate actions (pointer, keyboard) on this session.
    pub coordinates: Support,
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
    use crate::driver::SemanticOnly;
    use crate::semantic::{fake::FakeTree, SemanticSession};
    use crate::test_support::{session_driving, session_with};
    use std::sync::Arc;

    #[tokio::test]
    async fn semantic_support_follows_the_session() {
        let (plain, _) = session_with(None);
        let s = Support::semantic(&plain);
        assert!(!s.supported);
        assert!(s.reason.unwrap().contains("coordinates only"));
        let tree = Arc::new(FakeTree::new("A", 1));
        let (native, _) = session_with(Some(SemanticSession::new(tree.factory())));
        let s = Support::semantic(&native);
        assert!(s.supported && s.reason.is_none());
    }

    #[tokio::test]
    async fn coordinate_support_follows_the_driver() {
        let (plain, _) = session_with(None);
        let c = Support::coordinates(&plain);
        assert!(c.supported && c.reason.is_none());
        let wayland = session_driving(Arc::new(SemanticOnly("why".to_string())), None);
        let c = Support::coordinates(&wayland);
        assert!(!c.supported);
        assert_eq!(c.reason.as_deref(), Some("why"));
    }
}
