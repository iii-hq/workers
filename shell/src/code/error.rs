//! Worker-wide error type. Serialises to a `{"code":"C2xx","message":"..."}`
//! JSON string that handlers return via `Result<_, String>`; the engine
//! surfaces that string verbatim to callers.
//!
//! Codes mirror `shell::fs::*`'s `S2xx` scheme so consumers can pattern
//! against a stable prefix.
//!
//! MESSAGE STYLE — every error message must carry:
//!   (a) what happened (the input + actual values),
//!   (b) why it was rejected,
//!   (c) the corrective next call, with enough detail that an LLM agent can
//!       make a successful second call using ONLY the error text.
//!
//! REDACTION INVARIANT — C211 deliberately folds "not found" and "access
//! denied" into a single code and identical wording so callers cannot probe
//! for the existence of a protected file. Messages MUST NOT distinguish the
//! two cases and MUST NOT name filesystem paths the caller did not supply
//! (e.g. discovered child paths inside a recursive walk).

use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

/// Structured per-entry error as it appears on the wire.
///
/// Use `code` for stable programmatic branching (e.g. `"C211"` for
/// not-found-or-denied). `message` carries the human/LLM-readable
/// problem description plus the corrective next call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct WireError {
    /// Stable error code, e.g. "C211". See the README error table.
    pub code: String,
    /// Human/LLM-readable message: problem + actual values + corrective next call.
    pub message: String,
}

/// The one allowed C211 recovery-hint suffix. Both `not_found_or_denied`
/// and the `From<io::Error>` NotFound arm build from this const, so the
/// missing and glob-denied wordings can never drift apart.
const C211_SUFFIX: &str =
    "not found or not accessible. Verify the path with coder::list-folder or coder::tree.";

#[derive(Debug, Error, Serialize)]
#[serde(tag = "code", content = "message")]
pub enum CoderError {
    /// Malformed input: bad payload, illegal line numbers, overlapping ops, …
    #[error("C210: {0}")]
    #[serde(rename = "C210")]
    BadInput(String),

    /// Path not found OR matched a `non_accessible_globs` entry. Both are
    /// folded into the same code so callers can't probe for the existence
    /// of a denied file by toggling the glob.
    #[error("C211: {0}")]
    #[serde(rename = "C211")]
    NotFoundOrDenied(String),

    /// File exceeds `max_read_bytes` or `max_write_bytes`.
    #[error("C213: {0}")]
    #[serde(rename = "C213")]
    TooLarge(String),

    /// Path escapes every allowed root, lexically or through a symlink.
    #[error("C215: {0}")]
    #[serde(rename = "C215")]
    OutsideBase(String),

    /// Underlying I/O error.
    #[error("C216: {0}")]
    #[serde(rename = "C216")]
    Io(String),

    /// `create-file` saw an existing file and `overwrite=false`.
    #[error("C217: {0}")]
    #[serde(rename = "C217")]
    AlreadyExists(String),

    /// Path canonicalises inside a configured root but OUTSIDE the
    /// per-call `base_dir` the session is scoped to. Distinct from `C215`
    /// (outside EVERY root) so the rejection can name the session
    /// directory rather than contradict `coder::info`'s allowed-roots list.
    #[error("C218: {0}")]
    #[serde(rename = "C218")]
    OutsideSession(String),
}

impl CoderError {
    /// Render as a JSON object string suitable for handler return values.
    pub fn to_wire_string(&self) -> String {
        // serde_json on the enum produces `{"code":"C2xx","message":"..."}`
        // thanks to the `tag/content` attributes above.
        serde_json::to_string(self)
            .unwrap_or_else(|_| format!("{{\"code\":\"C216\",\"message\":\"{self}\"}}"))
    }

    pub fn code(&self) -> &'static str {
        match self {
            CoderError::BadInput(_) => "C210",
            CoderError::NotFoundOrDenied(_) => "C211",
            CoderError::TooLarge(_) => "C213",
            CoderError::OutsideBase(_) => "C215",
            CoderError::Io(_) => "C216",
            CoderError::AlreadyExists(_) => "C217",
            CoderError::OutsideSession(_) => "C218",
        }
    }

    /// The inner message string (without the `C2xx: ` prefix that the
    /// `Display` impl prepends). Used by `WireError` to populate the
    /// `message` field without duplicating the code prefix.
    pub fn message(&self) -> &str {
        match self {
            CoderError::BadInput(m)
            | CoderError::NotFoundOrDenied(m)
            | CoderError::TooLarge(m)
            | CoderError::OutsideBase(m)
            | CoderError::Io(m)
            | CoderError::AlreadyExists(m)
            | CoderError::OutsideSession(m) => m,
        }
    }

    /// Convert to the structured wire form used in per-entry batch results.
    pub fn to_wire_error(&self) -> WireError {
        WireError {
            code: self.code().to_string(),
            message: self.message().to_string(),
        }
    }

    /// The primary C211 wording. Single constructor so the missing and
    /// glob-denied cases can never drift apart (REDACTION INVARIANT).
    /// `not_found_or_denied_subtree` below is the ONLY other allowed
    /// C211 shape.
    pub fn not_found_or_denied(path: &str) -> Self {
        CoderError::NotFoundOrDenied(format!("{path}: {C211_SUFFIX}"))
    }

    /// The ONLY other allowed C211 shape: a recursive delete refused
    /// because the subtree contains non-accessible entries. Redaction-safe
    /// because non-accessible entries' EXISTENCE is already public by
    /// design — `list-folder`/`tree` show them with `non_accessible: true`
    /// flags; only their content/identity is protected — and this message
    /// names neither (`parent_path` is the path the CALLER supplied).
    pub fn not_found_or_denied_subtree(parent_path: &str) -> Self {
        CoderError::NotFoundOrDenied(format!(
            "{parent_path}: subtree contains non-accessible entries; \
             refusing recursive delete."
        ))
    }

    /// Map an `io::Error` from an operation on a caller-supplied `path`.
    /// EVERY arm names that path (MESSAGE STYLE: the error alone must tell
    /// the caller which input to act on): NotFound folds into the
    /// standardized C211 wording; all other kinds prefix the path onto the
    /// `From<io::Error>` mapping ("<path>: <os error text>"). Handlers MUST
    /// use this (not bare `?`) whenever the wire path is in scope.
    /// Redaction-safe by construction: `path` is caller-supplied at every
    /// call site, never a discovered filesystem entry.
    pub fn io_for_path(e: std::io::Error, path: &str) -> Self {
        match Self::from(e) {
            // From<io::Error> already standardized the C211 wording; rebuild
            // through the sanctioned constructor so the path is prefixed.
            CoderError::NotFoundOrDenied(_) => Self::not_found_or_denied(path),
            CoderError::AlreadyExists(m) => CoderError::AlreadyExists(format!("{path}: {m}")),
            CoderError::Io(m) => CoderError::Io(format!("{path}: {m}")),
            // From<io::Error> only produces the three variants above; keep
            // any future variants untouched rather than double-prefixing.
            other => other,
        }
    }
}

impl From<&CoderError> for WireError {
    fn from(e: &CoderError) -> Self {
        e.to_wire_error()
    }
}

impl From<std::io::Error> for CoderError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            // Defense in depth for the REDACTION INVARIANT: a bare `?` on a
            // fs op must NEVER leak raw OS text ("No such file or directory
            // (os error 2)") — that wording is distinguishable from the
            // glob-denied message and would let callers probe for protected
            // files. The raw error detail is deliberately dropped. Handlers
            // should prefer `CoderError::io_for_path` so the message also
            // names the caller-supplied path; this generic arm is the
            // fallback when no wire path is in scope.
            std::io::ErrorKind::NotFound => CoderError::NotFoundOrDenied(C211_SUFFIX.to_string()),
            std::io::ErrorKind::AlreadyExists => CoderError::AlreadyExists(e.to_string()),
            _ => CoderError::Io(e.to_string()),
        }
    }
}

pub fn err_to_string(e: CoderError) -> String {
    e.to_wire_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_input_serializes_with_code_and_message() {
        let s = CoderError::BadInput("nope".into()).to_wire_string();
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
        assert_eq!(v["code"], "C210");
        assert_eq!(v["message"], "nope");
    }

    #[test]
    fn io_not_found_maps_to_c211_without_raw_os_text() {
        let e: CoderError = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert_eq!(e.code(), "C211");
        let msg = e.to_string();
        // REDACTION INVARIANT: the generic From arm must not leak OS error
        // detail that would distinguish "missing" from "denied".
        assert!(
            !msg.contains("os error") && !msg.contains("No such file"),
            "raw OS text leaked: {msg}"
        );
        assert!(
            msg.contains("not found or not accessible"),
            "standardized wording missing: {msg}"
        );
    }

    #[test]
    fn io_for_path_not_found_uses_standardized_wording_with_path() {
        let e = CoderError::io_for_path(
            std::io::Error::from(std::io::ErrorKind::NotFound),
            "some/file.txt",
        );
        assert_eq!(e.code(), "C211");
        let msg = e.to_string();
        assert!(msg.contains("some/file.txt: not found or not accessible"));
        assert!(!msg.contains("os error"), "raw OS text leaked: {msg}");
    }

    #[test]
    fn io_for_path_non_not_found_maps_to_io_with_path_prefix() {
        let e = CoderError::io_for_path(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            "some/file.txt",
        );
        assert_eq!(e.code(), "C216");
        // MESSAGE STYLE: every io_for_path arm must name the caller path —
        // a bare "Directory not empty (os error 66)" tells the caller
        // nothing about which batch entry to act on.
        let msg = e.message();
        assert!(
            msg.starts_with("some/file.txt: "),
            "C216 via io_for_path must prefix the caller path: {msg}"
        );
    }

    #[test]
    fn io_for_path_already_exists_maps_to_c217_with_path_prefix() {
        let e = CoderError::io_for_path(
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists"),
            "some/file.txt",
        );
        assert_eq!(e.code(), "C217");
        assert!(
            e.message().starts_with("some/file.txt: "),
            "C217 via io_for_path must prefix the caller path: {}",
            e.message()
        );
    }

    #[test]
    fn io_already_exists_maps_to_c217() {
        let e: CoderError = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "x").into();
        assert_eq!(e.code(), "C217");
    }

    #[test]
    fn each_variant_has_distinct_code() {
        use std::collections::HashSet;
        let codes: HashSet<&str> = [
            CoderError::BadInput("a".into()).code(),
            CoderError::NotFoundOrDenied("a".into()).code(),
            CoderError::TooLarge("a".into()).code(),
            CoderError::OutsideBase("a".into()).code(),
            CoderError::Io("a".into()).code(),
            CoderError::AlreadyExists("a".into()).code(),
            CoderError::OutsideSession("a".into()).code(),
        ]
        .into_iter()
        .collect();
        assert_eq!(codes.len(), 7);
    }

    /// DRIFT PREVENTION: `to_wire_error()` (structured per-entry form)
    /// and `to_wire_string()` (top-level Result<_,String> form) are two
    /// renderings of the same error; their `code` and `message` must
    /// never diverge for any variant.
    #[test]
    fn wire_error_matches_wire_string_for_every_variant() {
        let variants = [
            CoderError::BadInput("bad input msg".into()),
            CoderError::NotFoundOrDenied("not found msg".into()),
            CoderError::TooLarge("too large msg".into()),
            CoderError::OutsideBase("outside base msg".into()),
            CoderError::Io("io msg".into()),
            CoderError::AlreadyExists("already exists msg".into()),
            CoderError::OutsideSession("outside session msg".into()),
        ];
        for v in &variants {
            let wire = v.to_wire_error();
            let s: serde_json::Value =
                serde_json::from_str(&v.to_wire_string()).expect("wire string is valid JSON");
            assert_eq!(
                wire.code,
                s["code"].as_str().unwrap(),
                "code drift for variant {v:?}"
            );
            assert_eq!(
                wire.message,
                s["message"].as_str().unwrap(),
                "message drift for variant {v:?}"
            );
        }
    }
}
