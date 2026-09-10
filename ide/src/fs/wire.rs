//! Serde mirrors of the engine daemon's `sandbox::fs::*` JSON shapes.
//! Authoritative reference: `crates/shell-proto/src/lib.rs:69-100`
//! (FsEntry, FsMatch, FsSedFileResult) in the motia repo.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mode: String,
    pub mtime: i64,
    pub is_symlink: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FsMatch {
    #[serde(alias = "file")]
    pub path: String,
    pub line: u64,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FsSedFileResult {
    #[serde(alias = "file")]
    pub path: String,
    pub replacements: u64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_entry_round_trips() {
        let e = FsEntry {
            name: "hello.txt".into(),
            is_dir: false,
            size: 5,
            mode: "0644".into(),
            mtime: 1700000000,
            is_symlink: false,
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: FsEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        // Field name parity with engine daemon — case matters.
        assert!(json.contains("\"is_dir\""));
        assert!(json.contains("\"is_symlink\""));
    }

    #[test]
    fn fs_match_round_trips() {
        let m = FsMatch {
            path: "/a".into(),
            line: 3,
            content: "x".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: FsMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn fs_entry_deserializes_upstream_format() {
        let upstream = r#"{"name":"a.txt","is_dir":false,"size":5,"mode":"0644","mtime":1700000000,"is_symlink":false}"#;
        let e: FsEntry = serde_json::from_str(upstream).unwrap();
        assert_eq!(e.name, "a.txt");
        assert!(!e.is_dir);
    }

    #[test]
    fn fs_match_deserializes_upstream_format() {
        let upstream = r#"{"path":"/x","line":7,"content":"hi"}"#;
        let m: FsMatch = serde_json::from_str(upstream).unwrap();
        assert_eq!(m.line, 7);
        assert_eq!(m.content, "hi");
    }

    #[test]
    fn fs_match_accepts_legacy_file_alias() {
        // Older peers send `file` instead of `path`.
        let legacy = r#"{"file":"/x","line":1,"content":"hi"}"#;
        let m: FsMatch = serde_json::from_str(legacy).unwrap();
        assert_eq!(m.path, "/x");
    }

    #[test]
    fn fs_sed_result_deserializes_upstream_format() {
        let upstream = r#"{"path":"/x","replacements":2,"success":true}"#;
        let r: FsSedFileResult = serde_json::from_str(upstream).unwrap();
        assert!(r.success);
        assert!(r.error.is_none());
    }

    #[test]
    fn fs_sed_result_with_error_round_trips() {
        let r = FsSedFileResult {
            path: "/x".into(),
            replacements: 0,
            success: false,
            error: Some("nope".into()),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: FsSedFileResult = serde_json::from_str(&j).unwrap();
        assert_eq!(r, back);
    }
}
