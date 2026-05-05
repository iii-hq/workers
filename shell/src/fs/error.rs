//! `FsError` carries an S2xx code + message, serializing as the same
//! `{ "code": "...", "message": "..." }` shape the engine daemon emits
//! so callers don't need to branch on backend.

use serde::Serialize;
use std::io;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FsError {
    pub code: &'static str,
    pub message: String,
}

impl FsError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Mirrors the engine daemon's `SandboxError::from_io` so wire codes
    /// match across host and sandbox backends.
    pub fn from_io(path: &str, err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => Self::new("S211", format!("path not found: {path}")),
            io::ErrorKind::AlreadyExists => {
                Self::new("S213", format!("path already exists: {path}"))
            }
            io::ErrorKind::PermissionDenied => Self::new("S215", format!("{path}: {err}")),
            // S212 (wrong file type) and S214 (dir not empty) are checked
            // at the op level — they don't map to stable `ErrorKind` variants.
            _ => Self::new("S216", format!("{path}: {err}")),
        }
    }

    /// `serde_json::to_string` on `&'static str` + `String` fields is
    /// effectively infallible (OOM only); `expect` so future changes that
    /// break the invariant fail loudly instead of producing malformed JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("FsError is always serializable")
    }
}

pub fn parse_mode(mode: &str) -> Result<u32, FsError> {
    if mode.is_empty() {
        return Err(FsError::new("S210", "invalid octal mode: (empty)"));
    }
    let stripped = mode.trim_start_matches('0');
    let s = if stripped.is_empty() { "0" } else { stripped };
    u32::from_str_radix(s, 8)
        .map_err(|_| FsError::new("S210", format!("invalid octal mode: {mode}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn from_io_maps_kinds() {
        let e = FsError::from_io("/x", io::Error::new(ErrorKind::NotFound, "nope"));
        assert_eq!(e.code, "S211");
        let e = FsError::from_io("/x", io::Error::new(ErrorKind::AlreadyExists, "dupe"));
        assert_eq!(e.code, "S213");
        let e = FsError::from_io("/x", io::Error::new(ErrorKind::PermissionDenied, "no"));
        assert_eq!(e.code, "S215");
        let e = FsError::from_io("/x", io::Error::other("io"));
        assert_eq!(e.code, "S216");
    }

    #[test]
    fn parse_mode_accepts_common_forms() {
        assert_eq!(parse_mode("0755").unwrap(), 0o755);
        assert_eq!(parse_mode("755").unwrap(), 0o755);
        assert_eq!(parse_mode("0").unwrap(), 0);
    }

    #[test]
    fn parse_mode_rejects_garbage() {
        let err = parse_mode("xyz").unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[test]
    fn parse_mode_rejects_empty_string() {
        let err = parse_mode("").unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("(empty)"));
    }

    #[test]
    fn to_json_round_trips() {
        let e = FsError::new("S211", "nope");
        let j = e.to_json();
        assert!(j.contains("\"code\":\"S211\""));
        assert!(j.contains("\"message\":\"nope\""));
    }
}
