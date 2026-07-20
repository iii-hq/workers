//! In-page overlays a human watching the streamed viewport can see: the
//! ghost cursor (where the agent is acting) and the session-status badge
//! (which session controls the tab and in what mode). Both are injected JS,
//! idempotent, and confined to fixed-position elements with a sentinel id so
//! they never collide with page content.

/// Mount (once) the ghost-cursor element and move it to `(x, y)`. `click`
/// flashes it. Injected on each act so the overlay follows the agent.
pub fn ghost_cursor_script(x: f64, y: f64, click: bool) -> String {
    format!(
        r#"(() => {{
  let c = document.getElementById('iii-ghost-cursor');
  if (!c) {{
    c = document.createElement('div');
    c.id = 'iii-ghost-cursor';
    c.style.cssText = 'position:fixed;width:18px;height:18px;margin:-9px 0 0 -9px;'
      + 'border-radius:50%;background:rgba(16,185,129,.6);border:2px solid #10b981;'
      + 'z-index:2147483646;pointer-events:none;transition:left .08s,top .08s,transform .1s';
    (document.body || document.documentElement).appendChild(c);
  }}
  c.style.left = {x} + 'px';
  c.style.top = {y} + 'px';
  if ({click}) {{
    c.style.transform = 'scale(1.8)';
    setTimeout(() => {{ const e = document.getElementById('iii-ghost-cursor'); if (e) e.style.transform = 'scale(1)'; }}, 120);
  }}
  return true;
}})()"#
    )
}

/// Mount or update the session-status badge. `mode` is a short label
/// (active, read-only, handoff pending). Removed by `remove_badge_script`.
pub fn badge_script(session_id: &str, mode: &str) -> String {
    format!(
        r#"(() => {{
  let b = document.getElementById('iii-session-badge');
  if (!b) {{
    b = document.createElement('div');
    b.id = 'iii-session-badge';
    b.style.cssText = 'position:fixed;bottom:8px;right:8px;z-index:2147483646;'
      + 'background:rgba(17,24,39,.85);color:#f9fafb;font:11px system-ui,sans-serif;'
      + 'padding:4px 8px;border-radius:6px;pointer-events:none';
    (document.body || document.documentElement).appendChild(b);
  }}
  b.textContent = {session_id:?} + ' · ' + {mode:?};
  return true;
}})()"#
    )
}

pub fn remove_badge_script() -> String {
    "(() => { const b = document.getElementById('iii-session-badge'); if (b) b.remove(); return true; })()".to_string()
}

pub fn remove_ghost_cursor_script() -> String {
    "(() => { const c = document.getElementById('iii-ghost-cursor'); if (c) c.remove(); return true; })()".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ghost_cursor_positions_and_flags_click() {
        let js = ghost_cursor_script(12.0, 34.0, true);
        assert!(js.contains("12 + 'px'"));
        assert!(js.contains("34 + 'px'"));
        assert!(js.contains("if (true)"));
        assert!(js.contains("iii-ghost-cursor"));
    }

    #[test]
    fn badge_renders_id_and_mode_as_literals() {
        let js = badge_script("b1", "read-only");
        assert!(js.contains("\"b1\""));
        assert!(js.contains("\"read-only\""));
    }
}
