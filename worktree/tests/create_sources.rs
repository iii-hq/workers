//! Create sources beyond a local ref: the GitHub pull-request head path
//! (against a file:// origin fixture), codename branch naming, and the
//! advisory dev port on the wire.

mod support;

use std::path::Path;

use support::{commit_file, git, init_repo, make_env, test_config};
use worktree::config::BranchNaming;
use worktree::functions::create;
use worktree::ids;

/// An origin repo publishing `refs/pull/7/head`, and a consumer clone with
/// it configured as `origin` over file://.
fn setup_pr_fixture(tmp: &Path) -> (std::path::PathBuf, String) {
    let origin = tmp.join("origin");
    init_repo(&origin);
    let pr_sha = commit_file(&origin, "pr.txt", "pr work\n", "pr work");
    git(&origin, &["update-ref", "refs/pull/7/head", &pr_sha]);
    // Rewind main so the PR ref is the only place the commit lives.
    git(&origin, &["reset", "--hard", "HEAD~1"]);

    let repo = tmp.join("repo");
    init_repo(&repo);
    let url = format!("file://{}", origin.display());
    git(&repo, &["remote", "add", "origin", &url]);
    (repo, pr_sha)
}

#[tokio::test]
async fn create_from_pr_head_branches_at_the_fetched_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, pr_sha) = setup_pr_fixture(tmp.path());
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let created = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: None,
            pr: Some(7),
            session_id: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(created.branch, "iii/pr-7");
    assert_eq!(created.base_ref, "refs/pull/7/head");
    assert_eq!(created.base_sha, pr_sha);
    let head = git(Path::new(&created.path), &["rev-parse", "HEAD"]);
    assert_eq!(head, pr_sha);
    assert!(Path::new(&created.path).join("pr.txt").is_file());
}

#[tokio::test]
async fn create_pr_is_mutually_exclusive_and_fails_cleanly_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let (repo, _) = setup_pr_fixture(tmp.path());
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let repo_path = repo.to_string_lossy().into_owned();

    let err = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo_path.clone(),
            base_ref: Some("main".into()),
            branch: None,
            pr: Some(7),
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W001");

    let err = create::handle(
        &env.deps,
        create::Request {
            repo_path,
            base_ref: None,
            branch: None,
            pr: Some(999),
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W112");
    assert!(
        err.message.contains("refs/pull/999/head"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn codename_naming_mints_the_deterministic_friendly_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let mut cfg = test_config(tmp.path());
    cfg.branch_naming = BranchNaming::Codename;
    let env = make_env(tmp.path(), cfg);

    let created = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: None,
            pr: None,
            session_id: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        created.branch,
        ids::codename_branch("iii/", &created.worktree_id)
    );
    let tail = created.branch.strip_prefix("iii/").unwrap();
    assert_eq!(tail.split('-').count(), 3, "{}", created.branch);
    let branches = git(&repo, &["branch", "--list", &created.branch]);
    assert!(branches.contains(tail));
}

#[tokio::test]
async fn dev_port_rides_the_create_response() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let created = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: None,
            pr: None,
            session_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(created.dev_port, ids::dev_port(&created.worktree_id));
    assert!((10_000..20_000).contains(&created.dev_port));
}
