//! Cross-cutting integration coverage for the folded `coder::*` batched
//! update applier. The per-module unit tests cover most branches; these
//! tests target the public handler surface and the atomicity invariants
//! spanning multiple files.

use std::path::PathBuf;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::update_file::{
    handle as update_handle, UpdateFileInput, UpdateFileSpec, UpdateOp,
};
use shell::code::path::PathResolver;
use tempfile::tempdir;

fn make(base: PathBuf, globs: Vec<&str>) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let cfg = Arc::new(CoderConfig {
        base_paths: vec![base],
        non_accessible_globs: globs.into_iter().map(String::from).collect(),
        max_read_bytes: 1024 * 1024,
        max_write_bytes: 1024 * 1024,
        ..CoderConfig::default()
    });
    let r = Arc::new(PathResolver::new(&cfg).unwrap());
    (r, cfg)
}

#[tokio::test]
async fn bottom_up_application_e2e() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "1\n2\n3\n4\n5\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), vec![]);
    let out = update_handle(
        r,
        c,
        UpdateFileInput {
            files: vec![UpdateFileSpec {
                path: "a.txt".into(),
                ops: vec![
                    UpdateOp::Insert {
                        at_line: 1,
                        content: "0".into(),
                    },
                    UpdateOp::Remove {
                        from_line: 2,
                        to_line: 4,
                    },
                    UpdateOp::UpdateLines {
                        from_line: 5,
                        to_line: 5,
                        content: "FIVE".into(),
                    },
                ],
            }],
            base_dir: None,
        },
    )
    .await
    .unwrap();
    assert!(out.results[0].success);
    let after = std::fs::read_to_string(tmp.path().join("a.txt")).unwrap();
    assert_eq!(after, "0\n1\nFIVE\n");
}

#[tokio::test]
async fn batch_with_mix_of_success_and_failure_preserves_originals() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("ok.txt"), "x\ny\n").unwrap();
    let original_ok = std::fs::read_to_string(tmp.path().join("ok.txt")).unwrap();
    std::fs::write(tmp.path().join("bad.txt"), "a\nb\nc\n").unwrap();
    let original_bad = std::fs::read_to_string(tmp.path().join("bad.txt")).unwrap();
    std::fs::write(tmp.path().join(".env"), "S=1\n").unwrap();
    let original_env = std::fs::read_to_string(tmp.path().join(".env")).unwrap();

    let (r, c) = make(tmp.path().to_path_buf(), vec!["**/.env"]);
    let out = update_handle(
        r,
        c,
        UpdateFileInput {
            files: vec![
                UpdateFileSpec {
                    path: "ok.txt".into(),
                    ops: vec![UpdateOp::Insert {
                        at_line: 1,
                        content: "P".into(),
                    }],
                },
                UpdateFileSpec {
                    path: "bad.txt".into(),
                    ops: vec![
                        UpdateOp::Remove {
                            from_line: 1,
                            to_line: 2,
                        },
                        UpdateOp::UpdateLines {
                            from_line: 2,
                            to_line: 3,
                            content: "Z".into(),
                        },
                    ],
                },
                UpdateFileSpec {
                    path: ".env".into(),
                    ops: vec![UpdateOp::Insert {
                        at_line: 1,
                        content: "X".into(),
                    }],
                },
            ],
            base_dir: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(out.results.len(), 3);
    assert!(out.results[0].success, "ok.txt should succeed");
    assert!(!out.results[1].success, "bad.txt overlap must be rejected");
    assert_eq!(out.results[1].error.as_ref().unwrap().code, "C210");
    assert!(!out.results[2].success, ".env must be denied");
    assert_eq!(out.results[2].error.as_ref().unwrap().code, "C211");

    assert_eq!(
        std::fs::read_to_string(tmp.path().join("ok.txt")).unwrap(),
        "P\n".to_string() + &original_ok
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("bad.txt")).unwrap(),
        original_bad
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(".env")).unwrap(),
        original_env
    );
}

#[tokio::test]
async fn crlf_line_endings_preserved_after_update() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("crlf.txt"), b"a\r\nb\r\nc\r\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), vec![]);
    update_handle(
        r,
        c,
        UpdateFileInput {
            files: vec![UpdateFileSpec {
                path: "crlf.txt".into(),
                ops: vec![UpdateOp::UpdateLines {
                    from_line: 2,
                    to_line: 2,
                    content: "B".into(),
                }],
            }],
            base_dir: None,
        },
    )
    .await
    .unwrap();
    let bytes = std::fs::read(tmp.path().join("crlf.txt")).unwrap();
    assert_eq!(bytes, b"a\r\nB\r\nc\r\n");
}

#[tokio::test]
async fn regex_replace_e2e() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "foo bar foo\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), vec![]);
    let out = update_handle(
        r,
        c,
        UpdateFileInput {
            files: vec![UpdateFileSpec {
                path: "a.txt".into(),
                ops: vec![UpdateOp::Replace {
                    pattern: "foo".into(),
                    replacement: "baz".into(),
                    ignore_case: false,
                    dot_matches_newline: false,
                    expect_matches: None,
                }],
            }],
            base_dir: None,
        },
    )
    .await
    .unwrap();
    assert!(out.results[0].success);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "baz bar baz\n"
    );
}
