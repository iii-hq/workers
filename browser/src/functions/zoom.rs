//! `browser::zoom` — page zoom like a browser's View menu: stepped levels
//! (50 … 200 %) applied as CSS zoom on the document root, so the viewport
//! and the screencast stay the same size and the page scales inside them.
//! The level belongs to the loaded document; the console re-applies it
//! after a navigation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The same ladder Chromium's zoom menu steps through.
pub const LEVELS: [u32; 11] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZoomInput {
    pub session_id: String,
    /// `in`, `out`, `reset`, or `set` (default when `level` is given).
    #[serde(default)]
    pub action: Option<String>,
    /// Explicit level in percent (50–200); snapped to the ladder.
    #[serde(default)]
    pub level: Option<u32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ZoomOutput {
    pub ok: bool,
    /// Level in percent now applied to the document.
    pub level: u32,
}

/// The ladder entry nearest to `level`.
pub fn snap(level: u32) -> u32 {
    LEVELS
        .iter()
        .copied()
        .min_by_key(|l| l.abs_diff(level))
        .unwrap_or(100)
}

/// Next level for `in` / `out` from `current`; the ladder ends stay put.
pub fn step(current: u32, direction_in: bool) -> u32 {
    let current = snap(current);
    let pos = LEVELS.iter().position(|l| *l == current).unwrap_or(5);
    let next = if direction_in {
        (pos + 1).min(LEVELS.len() - 1)
    } else {
        pos.saturating_sub(1)
    };
    LEVELS[next]
}

/// Reads the level the document currently has (100 when none was set).
pub fn read_script() -> &'static str {
    r#"(() => {
  const z = document.documentElement.style.zoom;
  const n = z === '' ? 1 : parseFloat(z);
  return Number.isFinite(n) && n > 0 ? Math.round(n * 100) : 100;
})()"#
}

pub fn apply_script(level: u32) -> String {
    format!(
        r#"(() => {{
  const root = document.documentElement;
  if (window.__iiiZoomOriginal === undefined) {{
    window.__iiiZoomOriginal = root.style.zoom || '';
  }}
  root.style.zoom = {level} === 100 ? window.__iiiZoomOriginal : String({level} / 100);
  return {level};
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_the_ladder() {
        assert_eq!(snap(100), 100);
        assert_eq!(snap(120), 125);
        assert_eq!(snap(5), 50);
        assert_eq!(snap(999), 200);
    }

    #[test]
    fn steps_and_stops_at_the_ends() {
        assert_eq!(step(100, true), 110);
        assert_eq!(step(100, false), 90);
        assert_eq!(step(200, true), 200);
        assert_eq!(step(50, false), 50);
        assert_eq!(step(123, true), 150);
    }
}
