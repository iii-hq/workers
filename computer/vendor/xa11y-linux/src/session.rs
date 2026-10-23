//! Linux desktop-session detection.
//!
//! A Wayland desktop commonly exports both `WAYLAND_DISPLAY` and `DISPLAY`;
//! the latter is the XWayland endpoint, not evidence that the desktop itself
//! is X11.  Every Linux capability asks this module for its own route.

use xa11y_core::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopBackend {
    X11,
    Wayland,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionEnvironment {
    display: bool,
    wayland_display: bool,
    session_type: Option<DesktopBackend>,
}

impl SessionEnvironment {
    fn current() -> Self {
        let session_type = std::env::var("XDG_SESSION_TYPE").ok().and_then(|value| {
            match value.to_ascii_lowercase().as_str() {
                "x11" => Some(DesktopBackend::X11),
                "wayland" => Some(DesktopBackend::Wayland),
                _ => None,
            }
        });
        Self {
            display: std::env::var_os("DISPLAY").is_some(),
            wayland_display: std::env::var_os("WAYLAND_DISPLAY").is_some(),
            session_type,
        }
    }
}

/// Select one backend for a capability.
///
/// An explicit `XDG_SESSION_TYPE` wins when its endpoint exists; otherwise a
/// Wayland endpoint wins over `DISPLAY`, because mixed sessions export the
/// latter for XWayland clients. No endpoint means an actionable unsupported
/// error rather than an attempted connection to a guessed backend.
pub(crate) fn select_backend(capability: &str) -> Result<DesktopBackend> {
    select_backend_from(SessionEnvironment::current(), capability)
}

fn select_backend_from(env: SessionEnvironment, capability: &str) -> Result<DesktopBackend> {
    Ok(match env.session_type {
        Some(DesktopBackend::Wayland) if env.wayland_display => DesktopBackend::Wayland,
        Some(DesktopBackend::X11) if env.display => DesktopBackend::X11,
        _ if env.wayland_display => DesktopBackend::Wayland,
        _ if env.display => DesktopBackend::X11,
        _ => {
            return Err(Error::Unsupported {
                feature: format!(
                    "{capability}: no usable desktop endpoint; set WAYLAND_DISPLAY or DISPLAY"
                ),
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{select_backend_from, DesktopBackend, SessionEnvironment};
    use xa11y_core::Error;

    fn env(display: bool, wayland_display: bool) -> SessionEnvironment {
        SessionEnvironment {
            display,
            wayland_display,
            session_type: None,
        }
    }

    #[test]
    fn auto_routes_pure_and_mixed_sessions() {
        assert_eq!(
            select_backend_from(env(true, false), "capture").unwrap(),
            DesktopBackend::X11
        );
        assert_eq!(
            select_backend_from(env(false, true), "capture").unwrap(),
            DesktopBackend::Wayland
        );
        assert_eq!(
            select_backend_from(env(true, true), "capture").unwrap(),
            DesktopBackend::Wayland
        );
        assert!(matches!(
            select_backend_from(env(false, false), "capture"),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn stale_session_type_does_not_override_a_live_endpoint() {
        let stale_wayland = SessionEnvironment {
            display: true,
            wayland_display: false,
            session_type: Some(DesktopBackend::Wayland),
        };
        assert_eq!(
            select_backend_from(stale_wayland, "capture").unwrap(),
            DesktopBackend::X11
        );
    }
}
