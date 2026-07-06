//! Integration coverage for the write journal + `coder::undo` /
//! `coder::checkpoints`: full pipeline through the real handlers.

use std::path::PathBuf;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::apply_patch::{handle as apply_handle, ApplyPatchInput};
use shell::code::functions::checkpoints::{handle as checkpoints_handle, CheckpointsInput};
use shell::code::functions::delete_file::{handle as delete_handle, DeleteFileInput};
use shell::code::functions::undo::{handle as undo_handle, UndoInput};
use shell::code::functions::update_file::{
    handle as update_handle, UpdateFileInput, UpdateFileSpec, UpdateOp,
};
use shell::code::path::PathResolver;
use shell::fs::FsScope;
use tempfile::tempdir;

fn make(base: PathBuf, journal_dir: PathBuf) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let mut cfg = CoderConfig {
        base_paths: vec![base],
        ..CoderConfig::default()
    };
    cfg.journal.dir = journal_dir.display().to_string();
    let cfg = Arc::new(cfg);
    let r = Arc::new(PathResolver::new(&cfg).unwrap());
    (r, cfg)
}

fn scope(root: &std::path::Path, turn: &str) -> Option<FsScope> {
    Some(FsScope {
        root: root.display().to_string(),
        grants: vec![],
        session_id: Some("s-e2e".into()),
        turn_id: Some(turn.into()),
    })
}

fn str_replace(path: &str, old: &str, new: &str, fs_scope: Option<FsScope>) -> UpdateFileInput {
    UpdateFileInput {
        files: vec![UpdateFileSpec {
            path: path.into(),
            ops: vec![UpdateOp::StrReplace {
                old_str: old.into(),
                new_str: new.into(),
                replace_all: false,
            }],
        }],
        fs_scope,
    }
}

#[tokio::test]
async fn undo_restores_an_update_byte_identical_and_redo_works() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "original\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());

    update_handle(
        r.clone(),
        c.clone(),
        str_replace("a.txt", "original", "changed", None),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "changed\n"
    );

    // Undo restores the before-image.
    let out = undo_handle(r.clone(), c.clone(), UndoInput::default())
        .await
        .unwrap();
    assert_eq!(out.undone.len(), 1);
    assert_eq!(out.undone[0].function_id, "coder::update-file");
    assert_eq!(out.undone[0].restored.len(), 1);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "original\n"
    );

    // Undo of the undo = redo.
    let out = undo_handle(r, c, UndoInput::default()).await.unwrap();
    assert_eq!(out.undone[0].function_id, "coder::undo");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "changed\n"
    );
}

#[tokio::test]
async fn undo_by_turn_reverts_everything_that_turn_did() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "a0\n").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "b0\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());

    // Turn t-1 edits a.txt and deletes b.txt; turn t-2 edits a.txt again.
    update_handle(
        r.clone(),
        c.clone(),
        str_replace("a.txt", "a0", "a1", scope(tmp.path(), "t-1")),
    )
    .await
    .unwrap();
    delete_handle(
        r.clone(),
        c.clone(),
        DeleteFileInput {
            paths: vec!["b.txt".into()],
            recursive: false,
            fs_scope: scope(tmp.path(), "t-1"),
        },
    )
    .await
    .unwrap();
    update_handle(
        r.clone(),
        c.clone(),
        str_replace("a.txt", "a1", "a2", scope(tmp.path(), "t-2")),
    )
    .await
    .unwrap();

    // Undo ONLY t-1: b.txt comes back; a.txt reverts to its t-1 before-image
    // (a0) because t-1's record is the oldest layer — t-2's record remains
    // journaled for its own undo.
    let out = undo_handle(
        r.clone(),
        c.clone(),
        UndoInput {
            steps: None,
            turn_id: Some("t-1".into()),
            fs_scope: scope(tmp.path(), "t-3"),
        },
    )
    .await
    .unwrap();
    assert_eq!(out.undone.len(), 2, "both t-1 records undone");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("b.txt")).unwrap(),
        "b0\n",
        "deleted file restored"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "a0\n"
    );
}

#[tokio::test]
async fn undo_removes_files_created_by_the_journaled_write() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());

    apply_handle(
        r.clone(),
        c.clone(),
        ApplyPatchInput {
            patch: "*** Begin Patch\n*** Add File: fresh.py\n+x = 1\n*** End Patch".into(),
            fs_scope: None,
        },
    )
    .await
    .unwrap();
    assert!(tmp.path().join("fresh.py").exists());

    let out = undo_handle(r, c, UndoInput::default()).await.unwrap();
    assert_eq!(out.undone[0].function_id, "coder::apply-patch");
    assert_eq!(
        out.undone[0].removed,
        vec![tmp
            .path()
            .canonicalize()
            .unwrap()
            .join("fresh.py")
            .display()
            .to_string()]
    );
    assert!(!tmp.path().join("fresh.py").exists());
}

#[tokio::test]
async fn checkpoints_lists_newest_first_with_turn_attribution() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "0\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());

    update_handle(
        r.clone(),
        c.clone(),
        str_replace("a.txt", "0", "1", scope(tmp.path(), "t-early")),
    )
    .await
    .unwrap();
    update_handle(
        r.clone(),
        c.clone(),
        str_replace("a.txt", "1", "2", scope(tmp.path(), "t-late")),
    )
    .await
    .unwrap();

    let out = checkpoints_handle(r, c, CheckpointsInput::default())
        .await
        .unwrap();
    assert_eq!(out.records.len(), 2);
    assert!(!out.truncated);
    assert_eq!(out.records[0].turn_id.as_deref(), Some("t-late"));
    assert_eq!(out.records[1].turn_id.as_deref(), Some("t-early"));
    assert_eq!(out.records[0].session_id.as_deref(), Some("s-e2e"));
    assert!(out.records[0].files[0].ends_with("a.txt"));
}

#[tokio::test]
async fn undo_reports_directory_delete_as_skipped_gap() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("dir")).unwrap();
    std::fs::write(tmp.path().join("dir/x.txt"), "x\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());

    delete_handle(
        r.clone(),
        c.clone(),
        DeleteFileInput {
            paths: vec!["dir".into()],
            recursive: true,
            fs_scope: None,
        },
    )
    .await
    .unwrap();

    let out = undo_handle(r, c, UndoInput::default()).await.unwrap();
    assert_eq!(out.undone[0].skipped.len(), 1, "dir delete is a gap");
    assert!(out.undone[0].restored.is_empty());
    assert!(
        !tmp.path().join("dir").exists(),
        "gaps are reported, not silently resurrected"
    );
}

#[tokio::test]
async fn undo_with_nothing_journaled_is_a_clear_error() {
    let tmp = tempdir().unwrap();
    let jd = tempdir().unwrap();
    let (r, c) = make(tmp.path().to_path_buf(), jd.path().to_path_buf());
    let err = undo_handle(r, c, UndoInput::default()).await.unwrap_err();
    assert!(err.contains("nothing to undo"), "{err}");
}
