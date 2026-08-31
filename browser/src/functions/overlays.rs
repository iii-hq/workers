//! In-page overlays a human watching the streamed viewport can see: the
//! ghost cursor (where the agent is acting) and the session-status badge
//! (which session controls the tab and in what mode). Both are injected JS,
//! idempotent, and confined to fixed-position elements with a sentinel id so
//! they never collide with page content.

/// Mount (once) the ghost-cursor element and move it to `(x, y)`. `click`
/// flashes it. Injected on each act so the overlay follows the agent.
pub fn ghost_cursor_script(x: f64, y: f64, click: bool) -> String {
    // An arrow pointer (lucide MousePointer2), the console's icon language,
    // with a white outline so it reads on any background. Its tip is the
    // top-left of the 24x24 box, so the element anchors there and the tip
    // lands on (x, y).
    format!(
        r#"(() => {{
  let c = document.getElementById('iii-ghost-cursor');
  if (!c) {{
    c = document.createElement('div');
    c.id = 'iii-ghost-cursor';
    c.style.cssText = 'position:fixed;width:22px;height:22px;margin:-2px 0 0 -2px;'
      + 'z-index:2147483646;pointer-events:none;transform-origin:4px 4px;'
      + 'filter:drop-shadow(0 1px 2px rgba(0,0,0,.45));'
      + 'transition:left .08s,top .08s,transform .1s';
    c.innerHTML = '<svg width="22" height="22" viewBox="0 0 24 24" '
      + 'fill="rgb(16,185,129)" stroke="rgb(255,255,255)" stroke-width="1.5" '
      + 'stroke-linejoin="round"><path d="M4 4 L11.5 20.5 L13.9 13.9 L20.5 11.5 Z"/>'
      + '</svg>';
    (document.body || document.documentElement).appendChild(c);
  }}
  c.style.left = {x} + 'px';
  c.style.top = {y} + 'px';
  if ({click}) {{
    c.style.transform = 'scale(1.35)';
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
