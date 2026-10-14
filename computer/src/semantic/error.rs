//! Semantic failures and how they reach the wire. Every refusal is an
//! `Error::Remote` with a stable code and a recovery hint in the message; the
//! codes deliberately avoid the harness's "outcome unknown" set (`timeout`,
//! `invocation_error`, ...), whose dispatch witness no later reply clears.

use iii_sdk::errors::Error;
use xa11y_core::Error as X;

/// macOS `kAXErrorInvalidUIElement`: the element is gone.
const AX_INVALID_UI_ELEMENT: i64 = -25202;
/// macOS `kAXErrorActionUnsupported` (xa11y usually maps it already).
const AX_ACTION_UNSUPPORTED: i64 = -25206;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    InvalidInput,
    InvalidSelector,
    StaleRef,
    NotSupported,
    Busy,
    PermissionDenied,
    UnsupportedSession,
    BackendError,
    EffectUnknown,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::InvalidInput => "invalid_input",
            Code::InvalidSelector => "invalid_selector",
            Code::StaleRef => "stale_ref",
            Code::NotSupported => "not_supported",
            Code::Busy => "busy",
            Code::PermissionDenied => "permission_denied",
            Code::UnsupportedSession => "unsupported_session",
            Code::BackendError => "backend_error",
            Code::EffectUnknown => "effect_unknown",
        }
    }

    fn recovery(self) -> &'static str {
        match self {
            Code::InvalidInput => "fix the request and retry",
            Code::InvalidSelector => "fix the selector and retry",
            Code::StaleRef => "re-run computer::find or computer::observe and use the new refs",
            Code::NotSupported => {
                "re-run computer::find (e.g. with [enabled=\"true\"] and timeout_ms) before acting"
            }
            Code::Busy => "another accessibility operation is running on this session; retry",
            Code::PermissionDenied if cfg!(target_os = "macos") => {
                "on macOS grant BOTH Accessibility and Screen Recording (System Settings › \
                 Privacy & Security) to the process running the computer worker, then retry"
            }
            Code::PermissionDenied => {
                "the OS refused accessibility access to the process running the computer \
                 worker; check its accessibility permissions, then retry"
            }
            Code::UnsupportedSession => {
                "act by coordinates, or start a native session (macOS, Windows or Linux)"
            }
            Code::BackendError => {
                "retry, or observe with include_image=true and act by coordinates"
            }
            Code::EffectUnknown => {
                "do not repeat the action; observe first to see whether it happened"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub code: Code,
    pub message: String,
    /// The scoped app is not running, or no app is in the foreground — yet,
    /// maybe: a timed find keeps waiting. Never on the wire.
    pub app_missing: bool,
}

impl SemanticError {
    pub fn new(code: Code, what: impl Into<String>) -> Self {
        Self::with_recovery(code, what, code.recovery())
    }

    /// A refusal with its own recovery hint instead of the code's generic
    /// one (a session without coordinates must not be told to act by them).
    pub fn with_recovery(code: Code, what: impl Into<String>, recovery: &str) -> Self {
        Self {
            code,
            message: format!("{}; {recovery}", what.into()),
            app_missing: false,
        }
    }

    /// Mark an `invalid_input` as "the app is not there (yet)".
    pub fn with_app_missing(mut self) -> Self {
        self.app_missing = true;
        self
    }

    /// What every semantic call gets on a remote or sandboxed session.
    pub fn coordinates_only() -> Self {
        Self::new(
            Code::UnsupportedSession,
            "this session drives a remote or sandboxed desktop by coordinates only",
        )
    }

    pub fn into_iii(self) -> Error {
        Error::Remote {
            code: self.code.as_str().to_string(),
            message: self.message,
            stacktrace: None,
        }
    }
}

/// An xa11y error named by its variant (plus the code of a `Platform`
/// error), never by its `Display`: `SelectorNotMatched` and `Timeout` carry
/// a `Diagnosis` built from provider data, and `AccessibilityNotEnabled` the
/// app's name — app text that skipped redaction and escaping.
fn kind(e: &X) -> String {
    let name = match e {
        X::Platform { code, .. } => return format!("Platform {code}"),
        X::PermissionDenied { .. } => "PermissionDenied",
        X::AccessibilityNotEnabled { .. } => "AccessibilityNotEnabled",
        X::SelectorNotMatched { .. } => "SelectorNotMatched",
        X::ElementStale { .. } => "ElementStale",
        X::ActionNotSupported { .. } => "ActionNotSupported",
        X::TextValueNotSupported => "TextValueNotSupported",
        X::Timeout { .. } => "Timeout",
        X::InvalidSelector { .. } => "InvalidSelector",
        X::InvalidActionData { .. } => "InvalidActionData",
        X::InvalidConfig { .. } => "InvalidConfig",
        X::NoElementBounds => "NoElementBounds",
        X::Unsupported { .. } => "Unsupported",
        _ => "unknown error",
    };
    name.to_string()
}

/// Classify a failed semantic action. `advertised` is whether the snapshot
/// listed the verb in the element's `actions`: on Windows a removed element
/// can no longer hand out the pattern it advertised, which xa11y reports as
/// `ActionNotSupported`, and no pattern method ran — so that is a stale ref.
pub fn map_act_error(e: &X, action: &str, advertised: bool) -> SemanticError {
    match e {
        X::PermissionDenied { instructions } => SemanticError::new(
            Code::PermissionDenied,
            format!("permission denied: {instructions}"),
        ),
        X::ElementStale { .. } => {
            SemanticError::new(Code::StaleRef, "the element no longer exists")
        }
        X::Platform { code, .. } if *code == AX_INVALID_UI_ELEMENT => {
            SemanticError::new(Code::StaleRef, "the element no longer exists")
        }
        X::ActionNotSupported { .. } if advertised => SemanticError::new(
            Code::StaleRef,
            format!(
                "the element advertised {action} but no longer accepts it (most likely removed)"
            ),
        ),
        X::ActionNotSupported { .. } => SemanticError::new(
            Code::NotSupported,
            format!("the element does not support {action}"),
        ),
        X::Platform { code, .. } if *code == AX_ACTION_UNSUPPORTED => SemanticError::new(
            Code::NotSupported,
            format!("the element does not support {action}"),
        ),
        X::Unsupported { .. } | X::InvalidActionData { .. } | X::NoElementBounds => {
            SemanticError::new(
                Code::NotSupported,
                format!("{action} is not available here ({})", kind(e)),
            )
        }
        // Everything else — Platform codes, TextValueNotSupported, variants
        // added later — may have happened after dispatch: conservative.
        other => SemanticError::new(
            Code::EffectUnknown,
            format!("{action} failed after it was dispatched ({})", kind(other)),
        ),
    }
}

/// Classify a failed read (snapshot or find). `rooted` is whether the element
/// that vanished is the read's own `root_ref` root: only then does
/// `ElementStale` mean the caller's ref is gone.
pub fn map_read_error(e: &X, rooted: bool) -> SemanticError {
    match e {
        X::PermissionDenied { instructions } => SemanticError::new(
            Code::PermissionDenied,
            format!("permission denied: {instructions}"),
        ),
        X::ElementStale { .. } if rooted => {
            SemanticError::new(Code::StaleRef, "the root_ref element no longer exists")
        }
        X::ElementStale { .. } => {
            SemanticError::new(Code::BackendError, "an element vanished during the read")
        }
        X::InvalidSelector { message, .. } => {
            SemanticError::new(Code::InvalidInput, message.clone())
        }
        other => SemanticError::new(
            Code::BackendError,
            format!("accessibility read failed ({})", kind(other)),
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn with_recovery_replaces_the_code_hint() {
        let e = SemanticError::with_recovery(Code::UnsupportedSession, "no screen", "read by ref");
        assert_eq!(e.code, Code::UnsupportedSession);
        assert_eq!(e.message, "no screen; read by ref");
        assert!(!e.app_missing);
    }

    use super::*;
    use xa11y_core::{Error as X, Role};

    fn remote_code(e: iii_sdk::errors::Error) -> String {
        match e {
            iii_sdk::errors::Error::Remote { code, .. } => code,
            other => panic!("expected Error::Remote, got {other:?}"),
        }
    }

    #[test]
    fn into_iii_keeps_the_code_and_adds_recovery() {
        let e = SemanticError::new(Code::StaleRef, "ref e1 is gone");
        assert!(e.message.starts_with("ref e1 is gone; "));
        assert!(e.message.contains("re-run computer::find"));
        assert_eq!(remote_code(e.into_iii()), "stale_ref");
    }

    #[test]
    fn act_errors_map_to_spec_codes() {
        let not_sup = X::ActionNotSupported {
            action: "press".into(),
            role: Role::Button,
        };
        assert_eq!(
            map_act_error(&not_sup, "invoke", false).code,
            Code::NotSupported
        );
        // Windows: a removed element cannot hand out its pattern.
        assert_eq!(map_act_error(&not_sup, "invoke", true).code, Code::StaleRef);
        let stale = X::ElementStale {
            selector: "handle:1".into(),
        };
        assert_eq!(map_act_error(&stale, "invoke", true).code, Code::StaleRef);
        let ax_dead = X::Platform {
            code: -25202,
            message: "x".into(),
        };
        assert_eq!(map_act_error(&ax_dead, "toggle", true).code, Code::StaleRef);
        let ax_unsup = X::Platform {
            code: -25206,
            message: "x".into(),
        };
        assert_eq!(
            map_act_error(&ax_unsup, "select", false).code,
            Code::NotSupported
        );
        let ax_cannot = X::Platform {
            code: -25204,
            message: "x".into(),
        };
        assert_eq!(
            map_act_error(&ax_cannot, "invoke", true).code,
            Code::EffectUnknown
        );
        assert_eq!(
            map_act_error(&X::TextValueNotSupported, "set_value", true).code,
            Code::EffectUnknown
        );
        assert_eq!(
            map_act_error(
                &X::Unsupported {
                    feature: "f".into()
                },
                "invoke",
                true
            )
            .code,
            Code::NotSupported
        );
        let denied = X::PermissionDenied {
            instructions: "grant AX".into(),
        };
        let e = map_act_error(&denied, "invoke", true);
        assert_eq!(e.code, Code::PermissionDenied);
        assert!(e.message.contains("grant AX"));
    }

    #[test]
    fn read_errors_map_to_spec_codes() {
        let denied = X::PermissionDenied {
            instructions: "grant".into(),
        };
        let e = map_read_error(&denied, false);
        assert_eq!(e.code, Code::PermissionDenied);
        if cfg!(target_os = "macos") {
            assert!(e.message.contains("Screen Recording"), "{}", e.message);
        } else {
            // The macOS grant steps would only mislead on another OS.
            assert!(
                !e.message.contains("macOS") && e.message.contains("refused accessibility access"),
                "{}",
                e.message
            );
        }
        let stale = X::ElementStale {
            selector: "s".into(),
        };
        // Only the root_ref root going away is a stale ref; anything else
        // vanishing mid-read is a backend hiccup.
        assert_eq!(map_read_error(&stale, true).code, Code::StaleRef);
        assert_eq!(map_read_error(&stale, false).code, Code::BackendError);
        assert_eq!(
            map_read_error(
                &X::InvalidSelector {
                    selector: "a\"b".into(),
                    message: "m".into()
                },
                false
            )
            .code,
            Code::InvalidInput
        );
        assert_eq!(
            map_read_error(
                &X::Platform {
                    code: 1,
                    message: "boom".into()
                },
                false
            )
            .code,
            Code::BackendError
        );
    }

    #[test]
    fn wire_codes_are_pinned_and_never_harness_unknown_outcome_codes() {
        let wire = [
            (Code::InvalidInput, "invalid_input"),
            (Code::InvalidSelector, "invalid_selector"),
            (Code::StaleRef, "stale_ref"),
            (Code::NotSupported, "not_supported"),
            (Code::Busy, "busy"),
            (Code::PermissionDenied, "permission_denied"),
            (Code::UnsupportedSession, "unsupported_session"),
            (Code::BackendError, "backend_error"),
            (Code::EffectUnknown, "effect_unknown"),
        ];
        for (c, s) in wire {
            assert_eq!(c.as_str(), s);
            assert!(
                ![
                    "timeout",
                    "invocation_error",
                    "invocation_stopped",
                    "engine_restart"
                ]
                .contains(&s),
                "{s}"
            );
        }
    }

    #[test]
    fn messages_never_carry_xa11y_display_text() {
        let diag = || xa11y_core::Diagnosis::new().last_observed("SENTINEL app text");
        let errs = [
            X::selector_not_matched("button").diagnose(diag()),
            X::timeout(std::time::Duration::from_secs(1)).diagnose(diag()),
            X::AccessibilityNotEnabled {
                app: "SENTINEL app".into(),
                instructions: "x".into(),
            },
            X::Platform {
                code: -25204,
                message: "SENTINEL os text".into(),
            },
            X::Unsupported {
                feature: "SENTINEL feature".into(),
            },
        ];
        for e in &errs {
            for m in [
                map_read_error(e, false).message,
                map_act_error(e, "invoke", true).message,
            ] {
                assert!(!m.contains("SENTINEL"), "{m}");
            }
        }
        assert!(
            map_read_error(&errs[1], false)
                .message
                .contains("(Timeout)"),
            "{}",
            map_read_error(&errs[1], false).message
        );
        assert!(map_read_error(&errs[3], false)
            .message
            .starts_with("accessibility read failed (Platform -25204)"));
        assert!(map_act_error(&errs[4], "invoke", true)
            .message
            .starts_with("invoke is not available here (Unsupported)"));
    }
}
