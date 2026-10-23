//! `browser::act` — click, type, press, scroll against a snapshot/pick ref
//! or raw viewport coordinates.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ActInput {
    pub session_id: String,
    /// `click`, `hover`, `type`, `select`, `press`, `scroll`, or `drag`.
    pub action: String,
    /// Mouse button for `click`: `left` (default), `right`, or `middle`.
    #[serde(default)]
    pub button: Option<String>,
    /// Clicks in the gesture: 2 double-clicks, 3 triple-clicks (`click`
    /// only; default 1, held to 1..3).
    #[serde(default)]
    pub click_count: Option<u32>,
    /// Element ref from `browser::elements` (`n4`), `browser::snapshot`
    /// (`e3`), `browser::dom::read` (`d7`) or `browser::picked` (`p1`). A
    /// click, type or select by ref is refused when the element is disabled,
    /// hidden or covered. Refs die on navigation; re-read the page after.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Viewport x, when acting by coordinates instead of ref.
    #[serde(default)]
    pub x: Option<f64>,
    /// Viewport y, when acting by coordinates instead of ref.
    #[serde(default)]
    pub y: Option<f64>,
    /// Text to insert (`type`). With a ref to an `<input>` or `<textarea>`
    /// it replaces the current value; otherwise it is inserted at the caret.
    #[serde(default)]
    pub text: Option<String>,
    /// Option to choose (`select`, native `<select>` only): its value or
    /// visible label.
    #[serde(default)]
    pub option: Option<String>,
    /// Key name for `press`: Enter, Tab, Escape, Backspace, Delete,
    /// ArrowUp/Down/Left/Right, Home, End, PageUp, PageDown.
    #[serde(default)]
    pub key: Option<String>,
    /// Scroll distance in pixels; positive scrolls down (`scroll`).
    /// Default 600, or 0 when `delta_x` is given.
    #[serde(default)]
    pub delta_y: Option<f64>,
    /// Horizontal scroll distance in pixels; positive scrolls right
    /// (`scroll`). Default 0.
    #[serde(default)]
    pub delta_x: Option<f64>,
    /// Drag end x, in viewport pixels (`drag`). The start is `x`/`y` or a
    /// `ref`; the end is `x2`/`y2`.
    #[serde(default)]
    pub x2: Option<f64>,
    /// Drag end y, in viewport pixels (`drag`).
    #[serde(default)]
    pub y2: Option<f64>,
    /// Draw the agent's ghost cursor where the action lands, for whoever
    /// watches the live view. Default true; false for a person's own input
    /// from a viewer, so the page shows no agent cursor.
    #[serde(default)]
    pub ghost_cursor: Option<bool>,
}

/// Most clicks one `click` makes (a triple-click selects a paragraph).
pub const MAX_CLICK_COUNT: u32 = 3;

impl ActInput {
    /// The clicks of a `click`, held to 1..`MAX_CLICK_COUNT`.
    pub fn clicks(&self) -> i64 {
        i64::from(self.click_count.unwrap_or(1).clamp(1, MAX_CLICK_COUNT))
    }

    /// The wheel deltas of a `scroll`: `(delta_x, delta_y)`. A bare scroll
    /// pages down 600px; one that only names `delta_x` moves sideways only.
    pub fn scroll_deltas(&self) -> (f64, f64) {
        let delta_x = self.delta_x.unwrap_or(0.0);
        let delta_y = self
            .delta_y
            .unwrap_or(if self.delta_x.is_some() { 0.0 } else { 600.0 });
        (delta_x, delta_y)
    }
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
    fn scroll_deltas_default_down_and_go_sideways_only_when_asked() {
        let scroll = |delta_x, delta_y| ActInput {
            action: "scroll".to_string(),
            delta_x,
            delta_y,
            ..ActInput::default()
        };
        assert_eq!(scroll(None, None).scroll_deltas(), (0.0, 600.0));
        assert_eq!(scroll(None, Some(-120.0)).scroll_deltas(), (0.0, -120.0));
        assert_eq!(scroll(Some(240.0), None).scroll_deltas(), (240.0, 0.0));
        assert_eq!(
            scroll(Some(-40.0), Some(80.0)).scroll_deltas(),
            (-40.0, 80.0)
        );
    }

    #[test]
    fn click_count_is_held_to_one_through_three() {
        let click = |click_count| ActInput {
            action: "click".to_string(),
            click_count,
            ..ActInput::default()
        };
        assert_eq!(click(None).clicks(), 1);
        assert_eq!(click(Some(0)).clicks(), 1);
        assert_eq!(click(Some(2)).clicks(), 2);
        assert_eq!(click(Some(1_000_000)).clicks(), 3);
    }

    #[test]
    fn known_keys_resolve_unknown_reject() {
        assert_eq!(key_spec("Enter").unwrap().windows_virtual_key_code, 13);
        assert_eq!(key_spec("PageDown").unwrap().code, "PageDown");
        assert!(key_spec("Meta+Shift+P").is_none());
    }
}
