//! Integration coverage for `coder::worktree-add` / `coder::worktree-remove`
//! — isolated sub-agent workspaces on real git repos.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::worktree::{
    handle_add, handle_remove, WorktreeAddInput, WorktreeRemoveInput,
};
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

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .status()
        .expect("git binary available in test env");
    assert!(status.success(), "git {args:?} failed");
}

fn seed_repo(root: &Path) {
    git(root, &["init", "-b", "main"]);
    std::fs::write(root.join("f.txt"), "base\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "seed"]);
}

fn add_input(name: &str) -> WorktreeAddInput {
    WorktreeAddInput {
        name: name.into(),
        fs_scope: None,
    }
}

fn remove_input(name: &str) -> WorktreeRemoveInput {
    WorktreeRemoveInput {
        name: name.into(),
        fs_scope: None,
    }
}

#[tokio::test]
async fn add_creates_isolated_worktree_on_branch() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());

    let out = handle_add(r, c, add_input("child-a1b2")).await.unwrap();
    assert!(out.path.ends_with(".worktrees/child-a1b2"), "{}", out.path);
    assert_eq!(out.branch, "wt/child-a1b2");
    let wt = Path::new(&out.path);
    assert!(wt.join("f.txt").exists(), "worktree has the checkout");

    // Edits in the worktree do not touch the main tree.
    std::fs::write(wt.join("f.txt"), "changed\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("f.txt")).unwrap(),
        "base\n"
    );
}

#[tokio::test]
async fn remove_clean_worktree_deletes_dir_and_merged_branch() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());
    let added = handle_add(r.clone(), c.clone(), add_input("done-x1"))
        .await
        .unwrap();

    let out = handle_remove(r, c, remove_input("done-x1")).await.unwrap();
    assert!(out.removed);
    assert!(!out.dirty);
    assert!(
        out.branch_deleted,
        "no new commits — wt branch is merged and deletable"
    );
    assert!(!Path::new(&added.path).exists());
}

#[tokio::test]
async fn remove_dirty_worktree_is_refused_and_left_in_place() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());
    let added = handle_add(r.clone(), c.clone(), add_input("busy-z9"))
        .await
        .unwrap();
    std::fs::write(Path::new(&added.path).join("wip.txt"), "uncommitted\n").unwrap();

    let out = handle_remove(r, c, remove_input("busy-z9")).await.unwrap();
    assert!(!out.removed);
    assert!(out.dirty);
    assert!(Path::new(&added.path).join("wip.txt").exists());
}

#[tokio::test]
async fn remove_keeps_unmerged_branch() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());
    let added = handle_add(r.clone(), c.clone(), add_input("work-q7"))
        .await
        .unwrap();
    let wt = PathBuf::from(&added.path);
    std::fs::write(wt.join("new.txt"), "child work\n").unwrap();
    git(&wt, &["add", "."]);
    git(&wt, &["commit", "-m", "child work"]);

    let out = handle_remove(r, c, remove_input("work-q7")).await.unwrap();
    assert!(out.removed, "committed (clean) worktree is removable");
    assert!(
        !out.branch_deleted,
        "unmerged branch must survive for the parent to merge"
    );
    // The parent can still merge the child's branch.
    git(tmp.path(), &["merge", "wt/work-q7"]);
    assert!(tmp.path().join("new.txt").exists());
}

#[tokio::test]
async fn add_outside_a_git_repo_is_rejected() {
    let tmp = tempdir().unwrap();
    let (r, c) = make(tmp.path().to_path_buf());
    let err = handle_add(r, c, add_input("nope")).await.unwrap_err();
    assert!(err.contains("git work tree"), "{err}");
}

#[tokio::test]
async fn invalid_names_are_rejected() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());
    for bad in ["../escape", "a/b", "", "has space"] {
        let err = handle_add(r.clone(), c.clone(), add_input(bad))
            .await
            .unwrap_err();
        assert!(err.contains("invalid worktree name"), "{bad:?}: {err}");
    }
}

#[tokio::test]
async fn duplicate_name_is_rejected_with_guidance() {
    let tmp = tempdir().unwrap();
    seed_repo(tmp.path());
    let (r, c) = make(tmp.path().to_path_buf());
    handle_add(r.clone(), c.clone(), add_input("dup-1"))
        .await
        .unwrap();
    let err = handle_add(r, c, add_input("dup-1")).await.unwrap_err();
    assert!(err.contains("different name"), "{err}");
}
