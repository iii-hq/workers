//! The integration check chain against real repositories: every detection
//! path (ancestor, no-added-changes, trees-match, merge-adds-nothing,
//! patch-id squash fallback) plus the diverged branch that must never read
//! integrated, and the prune behavior change for integrated-but-ahead
//! worktrees.

mod support;

use std::path::Path;

use support::{commit_file, git, init_repo, make_env, test_config, TestEnv};
use worktree::functions::{create, prune, status};

struct Setup {
    env: TestEnv,
    repo: std::path::PathBuf,
    worktree_id: String,
    wt_path: std::path::PathBuf,
}

/// Repo on `main` with one managed worktree based on it.
async fn setup(tmp: &Path) -> Setup {
    let repo = tmp.join("repo");
    init_repo(&repo);
    let env = make_env(tmp, test_config(tmp));
    let created = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: Some("main".into()),
            branch: None,
            session_id: None,
        },
    )
    .await
    .unwrap();
    Setup {
        env,
        repo,
        worktree_id: created.worktree_id,
        wt_path: std::path::PathBuf::from(created.path),
    }
}

async fn integration(s: &Setup) -> (bool, Option<String>) {
    let st = status::handle(
        &s.env.deps,
        status::Request {
            worktree_id: s.worktree_id.clone(),
        },
    )
    .await
    .unwrap();
    (st.status.integrated, st.status.integration_reason)
}

#[tokio::test]
async fn fresh_worktree_reads_same_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("same_commit"));
}

#[tokio::test]
async fn merged_branch_reads_ancestor() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    commit_file(&s.wt_path, "feature.txt", "feature\n", "feature");
    // True merge into main, then main moves on.
    git(
        &s.repo,
        &[
            "merge",
            "--no-ff",
            "-m",
            "merge feature",
            &format!("iii/{}", s.worktree_id),
        ],
    );
    commit_file(&s.repo, "after.txt", "after\n", "after merge");
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("ancestor"));
}

#[tokio::test]
async fn reverted_branch_reads_no_added_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    commit_file(&s.wt_path, "scratch.txt", "x\n", "add scratch");
    git(&s.wt_path, &["revert", "--no-edit", "HEAD"]);
    // Main moves so the branch is not an ancestor-equal.
    commit_file(&s.repo, "after.txt", "after\n", "main moves");
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("no_added_changes"));
}

#[tokio::test]
async fn cherry_picked_branch_reads_trees_match() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    let feature_sha = commit_file(&s.wt_path, "feature.txt", "feature\n", "feature");
    // Rebase-style landing: same change replayed onto main as a new commit
    // (-x alters the message so the replay never reproduces the same id).
    git(&s.repo, &["cherry-pick", "-x", &feature_sha]);
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("trees_match"));
}

#[tokio::test]
async fn squash_merge_reads_merge_adds_nothing_after_target_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    commit_file(&s.wt_path, "feature.txt", "feature\n", "feature a");
    commit_file(&s.wt_path, "feature2.txt", "more\n", "feature b");
    git(
        &s.repo,
        &["merge", "--squash", &format!("iii/{}", s.worktree_id)],
    );
    git(&s.repo, &["commit", "-m", "squash: feature"]);
    // Target moves on OTHER files: trees differ, but merging the branch
    // adds nothing.
    commit_file(&s.repo, "unrelated.txt", "u\n", "unrelated");
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("merge_adds_nothing"));
}

#[tokio::test]
async fn squash_merge_with_retouched_files_reads_patch_id_match() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    commit_file(&s.wt_path, "feature.txt", "feature\n", "feature");
    git(
        &s.repo,
        &["merge", "--squash", &format!("iii/{}", s.worktree_id)],
    );
    git(&s.repo, &["commit", "-m", "squash: feature"]);
    // The squashed file is touched AGAIN on main: the in-memory merge now
    // conflicts, so only the patch-id fallback can prove integration.
    commit_file(&s.repo, "feature.txt", "feature v2\n", "retouch feature");
    let (integrated, reason) = integration(&s).await;
    assert!(integrated);
    assert_eq!(reason.as_deref(), Some("patch_id_match"));
}

#[tokio::test]
async fn diverged_branch_is_not_integrated() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(tmp.path()).await;
    commit_file(&s.wt_path, "feature.txt", "feature\n", "unlanded feature");
    commit_file(&s.repo, "other.txt", "other\n", "main diverges");
    let (integrated, reason) = integration(&s).await;
    assert!(!integrated, "diverged branch must not read integrated");
    assert_eq!(reason, None);
}

#[tokio::test]
async fn prune_removes_integrated_but_ahead_and_keeps_diverged() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = test_config(tmp.path());
    cfg.prune_expire_hours = 0;
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), cfg);

    let mk = |branch: Option<String>| create::Request {
        repo_path: repo.to_string_lossy().into_owned(),
        base_ref: Some("main".into()),
        branch,
        session_id: None,
    };
    let landed = create::handle(&env.deps, mk(None)).await.unwrap();
    let diverged = create::handle(&env.deps, mk(None)).await.unwrap();

    // `landed` is squash-merged into main (ahead of base, yet integrated).
    commit_file(
        Path::new(&landed.path),
        "landed.txt",
        "landed\n",
        "landed work",
    );
    git(&repo, &["merge", "--squash", &landed.branch]);
    git(&repo, &["commit", "-m", "squash: landed work"]);

    // `diverged` carries genuinely unmerged work.
    commit_file(
        Path::new(&diverged.path),
        "diverged.txt",
        "d\n",
        "unmerged work",
    );

    let out = prune::handle(
        &env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.pruned, vec![landed.worktree_id.clone()]);
    assert!(!Path::new(&landed.path).exists());
    assert!(Path::new(&diverged.path).is_dir());
    let reasons: Vec<(String, String)> =
        out.skipped.into_iter().map(|s| (s.id, s.reason)).collect();
    assert!(reasons.contains(&(diverged.worktree_id.clone(), "unmerged work".into())));
}
