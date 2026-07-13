//! `browser::act` — click, type, press, scroll against a snapshot/pick ref
//! or raw viewport coordinates.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActInput {
    pub session_id: String,
    /// `click`, `hover`, `type`, `press`, or `scroll`.
    pub action: String,
    /// Mouse button for `click`: `left` (default), `right`, or `middle`.
    #[serde(default)]
    pub button: Option<String>,
    /// Clicks in the gesture: 2 double-clicks (`click` only, default 1).
    #[serde(default)]
    pub click_count: Option<u32>,
    /// Element ref from `browser::snapshot` (`e3`) or `browser::picked`
    /// (`p1`). Refs die on navigation; re-snapshot after.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Viewport x, when acting by coordinates instead of ref.
    #[serde(default)]
    pub x: Option<f64>,
    /// Viewport y, when acting by coordinates instead of ref.
    #[serde(default)]
    pub y: Option<f64>,
    /// Text to insert (`type`).
    #[serde(default)]
    pub text: Option<String>,
    /// Key name for `press`: Enter, Tab, Escape, Backspace, Delete,
    /// ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown.
    #[serde(default)]
    pub key: Option<String>,
    /// Scroll distance in pixels; positive scrolls down (`scroll`).
    #[serde(default)]
    pub delta_y: Option<f64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActOutput {
    pub ok: bool,
    /// What was done, for the transcript.
    pub detail: String,
}

pub struct KeySpec {
    pub key: &'static str,
    pub code: &'static str,
    pub windows_virtual_key_code: i64,
    pub text: Option<&'static str>,
}

/// Key names accepted by `press`.
pub fn key_spec(name: &str) -> Option<KeySpec> {
    let spec = match name {
        "Enter" => KeySpec {
            key: "Enter",
            code: "Enter",
            windows_virtual_key_code: 13,
            text: Some("\r"),
        },
        "Tab" => KeySpec {
            key: "Tab",
            code: "Tab",
            windows_virtual_key_code: 9,
            text: None,
        },
        "Escape" => KeySpec {
            key: "Escape",
            code: "Escape",
            windows_virtual_key_code: 27,
            text: None,
        },
        "Backspace" => KeySpec {
            key: "Backspace",
            code: "Backspace",
            windows_virtual_key_code: 8,
            text: None,
        },
        "Delete" => KeySpec {
            key: "Delete",
            code: "Delete",
            windows_virtual_key_code: 46,
            text: None,
        },
        "ArrowUp" => KeySpec {
            key: "ArrowUp",
            code: "ArrowUp",
            windows_virtual_key_code: 38,
            text: None,
        },
        "ArrowDown" => KeySpec {
            key: "ArrowDown",
            code: "ArrowDown",
            windows_virtual_key_code: 40,
            text: None,
        },
        "ArrowLeft" => KeySpec {
            key: "ArrowLeft",
            code: "ArrowLeft",
            windows_virtual_key_code: 37,
            text: None,
        },
        "ArrowRight" => KeySpec {
            key: "ArrowRight",
            code: "ArrowRight",
            windows_virtual_key_code: 39,
            text: None,
        },
        "Home" => KeySpec {
            key: "Home",
            code: "Home",
            windows_virtual_key_code: 36,
            text: None,
        },
        "End" => KeySpec {
            key: "End",
            code: "End",
            windows_virtual_key_code: 35,
            text: None,
        },
        "PageUp" => KeySpec {
            key: "PageUp",
            code: "PageUp",
            windows_virtual_key_code: 33,
            text: None,
        },
        "PageDown" => KeySpec {
            key: "PageDown",
            code: "PageDown",
            windows_virtual_key_code: 34,
            text: None,
        },
        _ => return None,
    };
    Some(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_keys_resolve_unknown_reject() {
        assert_eq!(key_spec("Enter").unwrap().windows_virtual_key_code, 13);
        assert_eq!(key_spec("PageDown").unwrap().code, "PageDown");
        assert!(key_spec("Meta+Shift+P").is_none());
    }
}
