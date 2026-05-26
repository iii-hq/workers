//! Worker-wide error type. Serialises to a `{"code":"C2xx","message":"..."}`
//! JSON string that handlers return via `Result<_, String>`; the engine
//! surfaces that string verbatim to callers.
//!
//! Codes mirror `shell::fs::*`'s `S2xx` scheme so consumers can pattern
//! against a stable prefix.

use serde::Serialize;
use thiserror::Error;

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

    /// Path escapes `base_path` lexically or through a symlink.
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
        }
    }
}

impl From<std::io::Error> for CoderError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => CoderError::NotFoundOrDenied(e.to_string()),
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
    fn io_not_found_maps_to_c211() {
        let e: CoderError = std::io::Error::new(std::io::ErrorKind::NotFound, "x").into();
        assert_eq!(e.code(), "C211");
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
        ]
        .into_iter()
        .collect();
        assert_eq!(codes.len(), 6);
    }
}
