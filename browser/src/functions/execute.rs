//! `browser::execute` — run a multi-step async script in the page with
//! helpers and cross-call session state. One call does what would otherwise
//! cost a round-trip per action.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Ceiling on the serialized envelope coming back from the page. A script
/// returning more than this gets an error asking for a summary instead of
/// flooding the transcript.
pub const MAX_ENVELOPE_BYTES: usize = 1_000_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteInput {
    pub session_id: String,
    /// Async JavaScript body run in the page. Top-level `await` and `return`
    /// work. In scope: `state` (JSON object persisted across execute calls
    /// for the session), `log(...)` (collected into the response), `sleep(ms)`,
    /// and `waitFor(selector, { timeout })`. Return plain JSON.
    pub code: String,
    /// Upper bound on the run; clamped to `max_timeout_ms`.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExecuteOutput {
    pub ok: bool,
    /// The script's return value when `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Exception text when not `ok`. A "context destroyed" error usually
    /// means the script navigated; split the script at the navigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `log(...)` output collected during the run, in order.
    pub logs: Vec<String>,
    /// Session state after the run; the next execute call sees this as
    /// `state`.
    pub state: serde_json::Value,
}

/// Wrap the user's code with the helper prelude and the state/log capture.
/// The page returns one JSON string envelope; the worker parses it, stores
/// `state` back on the session, and shapes `ExecuteOutput`.
pub fn wrap_code(code: &str, state_json: &str) -> String {
    format!(
        r#"(async () => {{
  let state = {state_json};
  const logs = [];
  const log = (...args) => {{
    if (logs.length >= 200) return;
    logs.push(args.map((a) => {{
      if (typeof a === 'string') return a;
      try {{ return JSON.stringify(a); }} catch {{ return String(a); }}
    }}).join(' ').slice(0, 2000));
  }};
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const waitFor = async (selector, opts = {{}}) => {{
    const timeout = opts.timeout ?? 10000;
    const start = Date.now();
    while (Date.now() - start < timeout) {{
      const el = document.querySelector(selector);
      if (el) return el;
      await sleep(100);
    }}
    throw new Error('waitFor timed out after ' + timeout + 'ms: ' + selector);
  }};
  // Serialize the envelope, never throwing: a circular or otherwise
  // non-serializable state or result must not turn into a missing envelope.
  // On failure, keep the logs and the error, drop `state` to null, and keep
  // the result only if it serializes on its own.
  const serialize = (ok, result, error) => {{
    try {{
      return JSON.stringify({{ ok, result, error, logs, state }});
    }} catch (e) {{
      let safeResult = null;
      try {{ JSON.stringify(result); safeResult = result; }} catch (_) {{}}
      return JSON.stringify({{
        ok: false,
        result: safeResult,
        error: (error ? error + '; ' : '') + 'value not JSON-serializable: ' + String(e),
        logs,
        state: null,
      }});
    }}
  }};
  let result;
  try {{
    result = await (async () => {{
{code}
    }})();
  }} catch (e) {{
    return serialize(false, undefined, String((e && e.stack) || e));
  }}
  return serialize(true, result, undefined);
}})()"#
    )
}

/// The envelope shape produced by `wrap_code` in the page.
#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    #[serde(default)]
    pub state: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_code_embeds_state_and_body() {
        let wrapped = wrap_code("return state.n;", "{\"n\":7}");
        // `let` (not `const`) so user code may reassign `state = ...`.
        assert!(wrapped.contains("let state = {\"n\":7};"));
        assert!(wrapped.contains("return state.n;"));
        assert!(wrapped.starts_with("(async () => {"));
    }

    #[test]
    fn envelope_parses_error_shape() {
        let e: Envelope =
            serde_json::from_str(r#"{"ok":false,"error":"boom","logs":["a"],"state":{}}"#).unwrap();
        assert!(!e.ok);
        assert_eq!(e.error.as_deref(), Some("boom"));
        assert_eq!(e.logs, vec!["a"]);
    }
}
