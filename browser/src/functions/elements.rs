//! `browser::elements` — the page's visible, enabled controls as an indexed
//! table with `n` refs, plus the visible text, from one in-page evaluation
//! (`elements.js`). Also the in-page input guard every ref-addressed
//! `browser::act` runs before dispatching input, and the short post-input
//! settle wait. Ported from browser-use/jev-ultrafast.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Reads the table; evaluates to `ElementsOutput` JSON (without
/// `generation`), or null while the document has no body.
pub const OBSERVE_JS: &str = include_str!("elements.js");

/// Most elements one table lists (`elements.js` enforces it; the rest are
/// counted in `omitted`). Also keeps every judge `choice` under its 255 cap.
pub const MAX_ELEMENTS: usize = 250;

// ponytail: the registry lives on `window`, so the page can see or tamper
// with it; move observation into an isolated world (Page.createIsolatedWorld)
// if a hostile page ever matters.

/// Runs with `this` = the target element (`Runtime.callFunctionOn`). Scrolls
/// it into view when it is outside the viewport. For `click`, `type` and
/// `select` it then refuses a detached, disabled, hidden, read-only (type) or
/// covered element, so input never lands on whatever sits on top of it.
/// `type` focuses and, for `<input>`/`<textarea>`, selects the current value
/// so the inserted text replaces it. `select` picks the enabled option whose
/// value or label matches and fires `input`/`change`. Returns `{error}` on
/// refusal.
pub const GUARD_FN: &str = r#"function (kind, option) {
  const e = this;
  if (!e.isConnected) return { error: 'the element is no longer in the page' };
  const doc = e.ownerDocument, win = doc.defaultView;
  let r = e.getBoundingClientRect();
  if (r.bottom <= 0 || r.right <= 0 || r.top >= win.innerHeight || r.left >= win.innerWidth) {
    e.scrollIntoView({ block: 'center', inline: 'nearest' });
    r = e.getBoundingClientRect();
  }
  if (kind !== 'click' && kind !== 'type' && kind !== 'select') return {};
  if (e.matches(':disabled') || e.closest('[aria-disabled="true"],[inert]')) return { error: 'the element is disabled' };
  if (typeof e.checkVisibility === 'function' && !e.checkVisibility({ checkVisibilityCSS: true })) return { error: 'the element is hidden' };
  if (kind === 'type' && (e.readOnly || e.getAttribute('aria-readonly') === 'true')) return { error: 'the element is read-only' };
  if (r.width > 0 && r.height > 0) {
    const hit = doc.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    const own = hit && (e.contains(hit) || hit.contains(e) || [...(e.labels || [])].some((l) => l.contains(hit)));
    if (!own) {
      const what = !hit ? 'nothing (outside the viewport)' : hit.tagName.toLowerCase() + (hit.id ? '#' + hit.id : '') +
        (typeof hit.className === 'string' && hit.className.trim() ? '.' + hit.className.trim().split(/\s+/)[0] : '');
      return { error: 'the element is covered by ' + what };
    }
  }
  if (kind === 'type') {
    e.focus();
    if ((e.tagName === 'INPUT' || e.tagName === 'TEXTAREA') && typeof e.select === 'function') {
      e.select();
      return { replaced: true };
    }
    return {};
  }
  if (kind === 'select') {
    if (e.tagName !== 'SELECT') return { error: 'select needs a <select> element; for a custom dropdown, click it and then click the option' };
    const want = String(option ?? '').trim();
    const enabled = [...e.options].filter((o) => !o.disabled && !o.closest('optgroup[disabled]'));
    const o = enabled.find((o) => o.value === want || o.label.trim() === want) ||
      enabled.find((o) => o.label.trim().toLowerCase() === want.toLowerCase());
    if (!o) return { error: 'no enabled option matches ' + JSON.stringify(want) + '; options: ' + enabled.slice(0, 20).map((o) => o.label.trim()).join(', ') };
    if (e.multiple) o.selected = true; else e.value = o.value;
    e.dispatchEvent(new Event('input', { bubbles: true }));
    e.dispatchEvent(new Event('change', { bubbles: true }));
    return { selected: o.label.trim() };
  }
  return {};
}"#;

/// Promise that resolves once the page had a chance to react to input: two
/// animation frames, capped at 50 ms. After typing into an ARIA combobox it
/// instead waits for a visible `[role=option]`, capped at 200 ms, so the
/// next read sees the suggestions.
pub fn settle_script(typed: bool) -> String {
    format!(
        r#"new Promise((resolve) => {{
  const a = document.activeElement;
  const combo = {typed} && !!a && a.getAttribute('role') === 'combobox';
  let frames = 0, done = false;
  const finish = () => {{ if (!done) {{ done = true; resolve(true); }} }};
  setTimeout(finish, combo ? 200 : 50);
  const options = () => {{
    const ids = (a.getAttribute('aria-controls') || a.getAttribute('aria-owns') || '').split(/\s+/).filter(Boolean);
    const roots = ids.length ? ids.map((id) => document.getElementById(id)).filter(Boolean) : [document];
    return roots.flatMap((root) => [...root.querySelectorAll('[role="option"]')]).some((o) => {{
      const r = o.getBoundingClientRect();
      return r.width && r.height && r.bottom > 0 && r.top < innerHeight && o.checkVisibility({{ checkVisibilityCSS: true }});
    }});
  }};
  const tick = () => {{
    if (done) return;
    if (++frames >= 2 && (!combo || options())) finish(); else requestAnimationFrame(tick);
  }};
  requestAnimationFrame(tick);
}})"#
    )
}

/// The registry id behind an `n` ref (`n12` → 12), or None for other refs.
pub fn registry_id(r: &str) -> Option<u64> {
    r.strip_prefix('n')?.parse().ok()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ElementsInput {
    pub session_id: String,
}

/// One control. `operations` says what `browser::act` can do with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Element {
    /// `n` ref `browser::act` accepts; valid while the element stays in this
    /// document.
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub role: String,
    /// Accessible name (aria-labelledby, aria-label, label, text, title, or
    /// placeholder).
    pub label: String,
    /// Current value of a field or dropdown; a set password reads `(set)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<String>,
    /// `click`, `type` and/or `select`.
    pub operations: Vec<String>,
    /// Enabled `<select>` option labels (first 50), for `act` `select`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Options beyond the listed 50; `select` still accepts them by label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more_options: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElementsOutput {
    pub url: String,
    pub title: String,
    /// Visible text in the viewport, up to 6000 characters.
    pub text: String,
    pub can_scroll_up: bool,
    pub can_scroll_down: bool,
    /// Visible, enabled controls inside the viewport, in document order.
    pub elements: Vec<Element>,
    /// Controls past the 250-element cap, not listed.
    pub omitted: u32,
    /// Document generation; navigation advances it and every `n` ref dies.
    #[serde(default)]
    pub generation: u64,
}

impl ElementsOutput {
    /// The same page for progress purposes: document, text and controls.
    pub fn same_page(&self, other: &Self) -> bool {
        self.url == other.url
            && self.text == other.text
            && self.elements == other.elements
            && self.can_scroll_up == other.can_scroll_up
            && self.can_scroll_down == other.can_scroll_down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_come_only_from_n_refs() {
        assert_eq!(registry_id("n12"), Some(12));
        assert_eq!(registry_id("e12"), None);
        assert_eq!(registry_id("n"), None);
        assert_eq!(registry_id("nx"), None);
    }

    #[test]
    fn page_json_deserializes_without_generation() {
        let out: ElementsOutput = serde_json::from_value(serde_json::json!({
            "url": "https://x.test/", "title": "t", "text": "hello",
            "can_scroll_up": false, "can_scroll_down": true, "omitted": 0,
            "elements": [{"ref": "n1", "role": "button", "label": "Save", "operations": ["click"]}]
        }))
        .unwrap();
        assert_eq!(out.elements[0].r#ref, "n1");
        assert_eq!(out.generation, 0);
        assert!(out.same_page(&out.clone()));
    }
}
