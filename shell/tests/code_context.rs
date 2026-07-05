//! Integration coverage for `coder::context` — the one-call workspace
//! snapshot. Exercises the real handler against a temp git repo and a
//! plain directory (git: null).

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::context::{handle as context_handle, ContextInput};
use shell::code::path::PathResolver;
use tempfile::tempdir;

fn make(base: PathBuf) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let cfg = Arc::new(CoderConfig {
        base_paths: vec![base],
        ..CoderConfig::default()
    });
    let r = Arc::new(PathResolver::new(&cfg).unwrap());
    (r, cfg)
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .status()
        .expect("git binary available in test env");
    assert!(status.success(), "git {args:?} failed");
}

#[tokio::test]
async fn context_reports_git_state_and_instruction_files() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    git(root, &["init", "-b", "main"]);
    std::fs::write(root.join("lib.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("AGENTS.md"), "# Conventions\nuse tabs\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    std::fs::write(root.join("dirty.txt"), "uncommitted\n").unwrap();

    let (r, c) = make(root.to_path_buf());
    let out = context_handle(r, c, ContextInput::default()).await.unwrap();

    assert!(!out.primary_root.is_empty());
    assert_eq!(out.base_paths[0], out.primary_root);
    assert!(!out.platform.os.is_empty() && !out.platform.arch.is_empty());

    let git_ctx = out.git.expect("temp dir is a git repo");
    assert_eq!(git_ctx.branch, "main");
    assert!(!git_ctx.status_truncated);
    assert!(
        git_ctx.status.iter().any(|l| l.contains("dirty.txt")),
        "porcelain status lists the uncommitted file: {:?}",
        git_ctx.status
    );
    assert_eq!(git_ctx.recent_commits.len(), 1);
    assert!(git_ctx.recent_commits[0].contains("initial"));

    assert_eq!(out.instruction_files.len(), 1);
    assert_eq!(out.instruction_files[0].path, "AGENTS.md");
    assert!(out.instruction_files[0].content.contains("use tabs"));
    assert!(!out.instruction_files[0].truncated);
}

#[tokio::test]
async fn context_on_plain_directory_has_null_git_and_no_files() {
    let tmp = tempdir().unwrap();
    let (r, c) = make(tmp.path().to_path_buf());
    let out = context_handle(r, c, ContextInput::default()).await.unwrap();
    assert!(out.git.is_none());
    assert!(out.instruction_files.is_empty());
}

#[tokio::test]
async fn context_caps_oversized_instruction_file() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("CLAUDE.md"), "x".repeat(20 * 1024)).unwrap();
    let (r, c) = make(root.to_path_buf());
    let out = context_handle(r, c, ContextInput::default()).await.unwrap();
    assert_eq!(out.instruction_files.len(), 1);
    assert_eq!(out.instruction_files[0].path, "CLAUDE.md");
    assert!(out.instruction_files[0].truncated);
    assert_eq!(out.instruction_files[0].content.len(), 16 * 1024);
}
