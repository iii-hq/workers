//! `computer::act` — pointer and keyboard input against the desktop, by
//! coordinate. Coordinates are integer pixels, top-left origin, in the space
//! of the screenshot (`computer::screenshot` / the live frame).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActInput {
    pub session_id: String,
    /// What to do: `click`, `right_click`, `double_click`, `move`, `drag`,
    /// `scroll`, `type`, `press`, or `hotkey`.
    pub action: String,
    /// Target x (pixels). Required for click/right_click/double_click/move and
    /// the drag start; the scroll anchor defaults to screen center.
    #[serde(default)]
    pub x: Option<i64>,
    /// Target y (pixels).
    #[serde(default)]
    pub y: Option<i64>,
    /// Drag end x (with `to_y`), for `action=drag`.
    #[serde(default)]
    pub to_x: Option<i64>,
    /// Drag end y.
    #[serde(default)]
    pub to_y: Option<i64>,
    /// Mouse button for `drag` (`left`, `right`, `middle`). Default `left`.
    #[serde(default)]
    pub button: Option<String>,
    /// Text to type, for `action=type`.
    #[serde(default)]
    pub text: Option<String>,
    /// Keys for `press`/`hotkey`: a single key (`["enter"]`) or a chord
    /// (`["ctrl", "c"]`).
    #[serde(default)]
    pub keys: Option<Vec<String>>,
    /// Horizontal scroll amount for `action=scroll`. Default 0.
    #[serde(default)]
    pub scroll_x: Option<i64>,
    /// Vertical scroll amount for `action=scroll`; positive scrolls down.
    /// Default 3.
    #[serde(default)]
    pub scroll_y: Option<i64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActOutput {
    pub ok: bool,
    /// What was done, for the transcript.
    pub detail: String,
}
