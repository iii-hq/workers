//! `browser::handoff` and `browser::handoff::confirm` — pause a session for
//! a step only a human can do (CAPTCHA, 2FA, payment). The handoff call
//! parks until a human confirms in-page or a confirm call resolves it, or
//! the timeout elapses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Poll interval for the in-page continue control while parked.
pub const POLL_INTERVAL_MS: u64 = 250;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HandoffInput {
    pub session_id: String,
    /// What the human must do before the call continues. Shown in the
    /// in-page banner and the handoff-requested event.
    pub instructions: String,
    /// Give up after this long and return timed_out. Defaults to the config
    /// default; clamped to `max_timeout_ms`. Set generously; a human is slow.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HandoffOutput {
    /// True when a human confirmed; false when the wait timed out.
    pub confirmed: bool,
    pub handoff_id: String,
    /// How the confirmation arrived: `in_page`, `confirm_call`, or `timeout`.
    pub via: String,
    /// The page URL when the wait ended, so the caller can verify the step
    /// actually landed (human acknowledgment is not proof).
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HandoffConfirmInput {
    /// Confirm a specific handoff by id. Omit to confirm the one pending
    /// handoff for `session_id`.
    #[serde(default)]
    pub handoff_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HandoffConfirmOutput {
    /// True when a pending handoff matched and was resolved.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
}

/// JS that mounts the in-page continue banner and exposes a global flag the
/// worker polls. Idempotent per handoff id. `{:?}` renders both the id and
/// the instructions as valid double-quoted JS string literals, so no manual
/// escaping is needed and injection through the text is not possible.
pub fn banner_script(handoff_id: &str, instructions: &str) -> String {
    format!(
        r#"(() => {{
  window.__iiiHandoff = window.__iiiHandoff || {{}};
  const id = {handoff_id:?};
  if (document.getElementById('iii-handoff-' + id)) return;
  const bar = document.createElement('div');
  bar.id = 'iii-handoff-' + id;
  bar.setAttribute('role', 'dialog');
  bar.setAttribute('aria-label', 'Action needed');
  bar.style.cssText = 'position:fixed;top:0;left:0;right:0;z-index:2147483647;'
    + 'background:#1f2937;color:#f9fafb;font:14px system-ui,sans-serif;'
    + 'padding:12px 16px;display:flex;gap:12px;align-items:center;'
    + 'box-shadow:0 2px 8px rgba(0,0,0,.3)';
  const msg = document.createElement('span');
  msg.style.flex = '1';
  msg.textContent = 'Action needed: ' + {instructions:?};
  const btn = document.createElement('button');
  btn.textContent = 'Continue';
  btn.style.cssText = 'background:#10b981;color:#04231a;border:0;border-radius:6px;'
    + 'padding:8px 16px;font-weight:600;cursor:pointer';
  btn.addEventListener('click', () => {{ window.__iiiHandoff[id] = true; bar.remove(); }});
  bar.appendChild(msg);
  bar.appendChild(btn);
  document.body.appendChild(bar);
}})()"#
    )
}

/// JS that reads and clears the confirm flag for a handoff id.
pub fn poll_script(handoff_id: &str) -> String {
    format!(
        "(() => {{ const f = window.__iiiHandoff && window.__iiiHandoff[{handoff_id:?}]; return !!f; }})()"
    )
}

/// JS that removes the banner for a handoff id (on confirm-call or timeout).
pub fn remove_script(handoff_id: &str) -> String {
    format!(
        "(() => {{ const el = document.getElementById('iii-handoff-' + {handoff_id:?}); if (el) el.remove(); return true; }})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_renders_instructions_as_a_quoted_literal() {
        let js = banner_script("h1", "click \"here\" now");
        // {:?} escapes the embedded quotes so the JS literal stays valid.
        assert!(js.contains(r#"\"here\""#), "{js}");
        assert!(js.contains("iii-handoff-"));
        assert!(js.contains("__iiiHandoff[id] = true"));
    }

    #[test]
    fn poll_and_remove_reference_the_id() {
        assert!(poll_script("h7").contains("\"h7\""));
        assert!(remove_script("h7").contains("\"h7\""));
    }
}
