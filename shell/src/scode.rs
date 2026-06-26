//! Shared inbound S-code mapping for the sandbox-target backends.
//!
//! Both `fs::sandbox` and `exec::sandbox` forward ops to the engine
//! daemon and must translate the engine's error envelope (an
//! `Error`, or a stringified `{code,message}` payload) back into a
//! worker error type. The translation logic — scanning for an S-code,
//! canonicalizing it against the known table, and lifting an
//! `Error` into a worker error — was previously duplicated in both
//! modules. The two copies of the canonical-code table had already
//! diverged: each path recognized a different subset of S-codes, so
//! the same engine code could canonicalize to its specific value on
//! one path while collapsing to the generic `S216` on the other.
//!
//! This module unifies all three helpers behind one
//! constructor-agnostic surface. `map_iii_err` is generic over the
//! error constructor, so `fs` passes `FsError::new` and `exec` passes
//! `ExecError::new`; both share the SAME canonical table — the union
//! of every code either path legitimately handles.

use iii_sdk::errors::Error;
use serde_json::Value;

/// The generic fallback code for any engine error whose S-code we do
/// not recognize (or cannot recover at all).
pub const FALLBACK_CODE: &str = "S216";

/// Canonicalize an engine-supplied S-code string into a `&'static str`
/// from the one unified table. Unknown codes collapse to
/// [`FALLBACK_CODE`].
///
/// The table is the union of the codes the fs and exec sandbox paths
/// each legitimately handle:
/// - shared lifecycle/config codes: `S001`–`S004`, `S200`, `S210`, `S216`
/// - fs-specific codes: `S211`–`S215`, `S217`–`S220`
/// - exec-specific codes: `S100`–`S102`, `S300`, `S400`
///
/// Mapping every code on BOTH paths is intentional: the worker should
/// never silently downgrade an engine code to `S216` just because it
/// arrived over the "other" path.
pub fn map_static_code(code: &str) -> &'static str {
    match code {
        // Shared: validation / lifecycle / config + generic fallback.
        "S001" => "S001",
        "S002" => "S002",
        "S003" => "S003",
        "S004" => "S004",
        "S200" => "S200",
        "S210" => "S210",
        "S216" => "S216",
        // exec-specific: image / VM / resource codes.
        "S100" => "S100",
        "S101" => "S101",
        "S102" => "S102",
        "S300" => "S300",
        "S400" => "S400",
        // fs-specific: path / permission / payload codes.
        "S211" => "S211",
        "S212" => "S212",
        "S213" => "S213",
        "S214" => "S214",
        "S215" => "S215",
        "S217" => "S217",
        "S218" => "S218",
        "S219" => "S219",
        // base_dir (per-call session-scope) containment violation.
        "S220" => "S220",
        _ => FALLBACK_CODE,
    }
}

/// Scan a free-form string for the first standalone S-code (`S` + 3
/// digits) and return it. Used to recover a code from an
/// `invocation_failed` wrapper message or a raw handler string.
///
/// Matches preceded by an alphanumeric or `_` are rejected so we don't
/// grab the tail of a longer identifier (e.g. `FOO_S211`).
pub fn scan_s_code(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    // The window is 4 bytes (`bytes[i..=i+3]`), so the last valid `i` is
    // `len - 4`. The loop bound `< len - 3` gives `i <= len - 4`.
    // `saturating_sub(3)` collapses to 0 when `len < 4`, which yields an
    // empty range (no false access) on too-short inputs.
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i] == b'S'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
        {
            // Reject matches glued to a longer identifier on EITHER side so we
            // don't grab a substring of it: "FOO_S211" (left) or "S211foo" /
            // "S2119" (right) are not standalone S-codes.
            let preceded_by_word =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let followed_by_word = i + 4 < bytes.len()
                && (bytes[i + 4].is_ascii_alphanumeric() || bytes[i + 4] == b'_');
            if !preceded_by_word && !followed_by_word {
                return Some(&s[i..i + 4]);
            }
        }
    }
    None
}

/// Recover a canonical S-code from an `Error` and lift it into the
/// caller's error type via `make`.
///
/// Engine `Remote` errors carry the code structurally; the engine's
/// `invocation_failed` wrapper hides it inside the message string; mock
/// paths emit a JSON payload as `Handler(string)`. Unknown shapes fall
/// through to [`FALLBACK_CODE`].
///
/// Generic over the error constructor so both `FsError::new` and
/// `ExecError::new` reuse the identical recovery logic; the only
/// difference between the fs and exec paths is which `make` is passed.
pub fn map_iii_err<E>(err: &Error, make: impl Fn(&'static str, String) -> E) -> E {
    match err {
        Error::Remote { code, message, .. } if code.starts_with('S') => {
            return make(
                map_static_code(code),
                format!("forwarded from engine: {message}"),
            );
        }
        Error::Remote { message, .. } => {
            if let Some(c) = scan_s_code(message) {
                return make(
                    map_static_code(c),
                    format!("forwarded from engine (wrapped): {message}"),
                );
            }
        }
        Error::Handler(s) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(s) {
                if let Some(c) = parsed.get("code").and_then(|v| v.as_str()) {
                    let msg = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    return make(map_static_code(c), format!("forwarded from engine: {msg}"));
                }
            }
            if let Some(c) = scan_s_code(s) {
                return make(
                    map_static_code(c),
                    format!("forwarded from engine (raw): {s}"),
                );
            }
        }
        _ => {}
    }
    make(FALLBACK_CODE, format!("engine error: {err:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative engine code set, including every code that was
    /// previously divergent between the fs and exec `map_static_code`
    /// tables, plus the shared codes and an unknown one.
    ///
    /// `expected == input` means the code is canonical (passes through);
    /// the only collapsing case is the unknown `S999` -> `S216`.
    fn canonical_cases() -> Vec<(&'static str, &'static str)> {
        vec![
            // Shared across both paths.
            ("S001", "S001"),
            ("S002", "S002"),
            ("S003", "S003"),
            ("S004", "S004"),
            ("S200", "S200"),
            ("S210", "S210"),
            ("S216", "S216"),
            // Previously exec-only — now recognized on the fs path too.
            ("S100", "S100"),
            ("S101", "S101"),
            ("S102", "S102"),
            ("S300", "S300"),
            ("S400", "S400"),
            // Previously fs-only — now recognized on the exec path too.
            ("S211", "S211"),
            ("S212", "S212"),
            ("S213", "S213"),
            ("S214", "S214"),
            ("S215", "S215"),
            ("S217", "S217"),
            ("S218", "S218"),
            ("S219", "S219"),
            ("S220", "S220"),
            // Unknown collapses to the generic fallback.
            ("S999", "S216"),
        ]
    }

    #[test]
    fn map_static_code_unified_table_is_canonical() {
        for (input, expected) in canonical_cases() {
            assert_eq!(
                map_static_code(input),
                expected,
                "map_static_code({input}) should canonicalize to {expected}",
            );
        }
    }

    /// The core of the fix: the same engine code must produce the SAME
    /// mapped code regardless of which path (fs vs exec) handled it.
    /// We model each path by the constructor it passes to `map_iii_err`
    /// and assert both yield identical codes for identical inputs —
    /// including the codes that used to diverge.
    #[test]
    fn fs_and_exec_paths_agree_on_every_code() {
        // Two distinct "constructors" standing in for FsError::new and
        // ExecError::new. They differ only in a tag, mirroring how the
        // real types differ only in their constructor.
        let fs_make = |code: &'static str, msg: String| ("fs", code, msg);
        let exec_make = |code: &'static str, msg: String| ("exec", code, msg);

        for (input, expected) in canonical_cases() {
            let remote = Error::Remote {
                code: input.to_string(),
                message: "boom".into(),
                stacktrace: None,
            };
            let (fs_tag, fs_code, _) = map_iii_err(&remote, fs_make);
            let (exec_tag, exec_code, _) = map_iii_err(&remote, exec_make);
            assert_eq!(fs_tag, "fs");
            assert_eq!(exec_tag, "exec");
            assert_eq!(
                fs_code, exec_code,
                "fs and exec must map {input} to the same code",
            );
            assert_eq!(
                fs_code, expected,
                "{input} must canonicalize to {expected} on both paths",
            );
        }
    }

    #[test]
    fn scan_s_code_finds_standalone_code() {
        assert_eq!(scan_s_code("handler error: S211: not found"), Some("S211"));
        assert_eq!(scan_s_code("something broke S217 bad regex"), Some("S217"));
    }

    #[test]
    fn scan_s_code_rejects_glued_identifier() {
        assert_eq!(scan_s_code("fooS211bar"), None);
        assert_eq!(scan_s_code("FOO_S211"), None);
        // Right-hand boundary: a trailing alphanumeric/underscore means the
        // 3-digit run is part of a longer token, not a standalone S-code.
        assert_eq!(scan_s_code("S211foo"), None);
        assert_eq!(scan_s_code("S2119"), None);
        assert_eq!(scan_s_code("S211_x"), None);
        // But normal punctuation/whitespace boundaries still match.
        assert_eq!(scan_s_code("S211: not found"), Some("S211"));
        assert_eq!(scan_s_code("error S217."), Some("S217"));
    }

    #[test]
    fn scan_s_code_handles_short_inputs() {
        assert_eq!(scan_s_code(""), None);
        assert_eq!(scan_s_code("S21"), None);
    }

    #[test]
    fn map_iii_err_recovers_from_wrapped_message() {
        let err = Error::Remote {
            code: "invocation_failed".into(),
            message: "handler error: S211: not found".into(),
            stacktrace: None,
        };
        let (code, msg) = map_iii_err(&err, |c, m| (c, m));
        assert_eq!(code, "S211");
        assert!(msg.contains("wrapped"));
    }

    #[test]
    fn map_iii_err_recovers_from_handler_json() {
        let err = Error::Handler(r#"{"code":"S214","message":"directory not empty"}"#.into());
        let (code, _) = map_iii_err(&err, |c, m| (c, m));
        assert_eq!(code, "S214");
    }

    #[test]
    fn map_iii_err_recovers_from_handler_raw_scan() {
        let err = Error::Handler("something broke S217 bad regex".into());
        let (code, _) = map_iii_err(&err, |c, m| (c, m));
        assert_eq!(code, "S217");
    }

    #[test]
    fn map_iii_err_unknown_falls_back_to_s216() {
        let err = Error::Handler("just a plain old string".into());
        let (code, _) = map_iii_err(&err, |c, m| (c, m));
        assert_eq!(code, "S216");
    }
}
