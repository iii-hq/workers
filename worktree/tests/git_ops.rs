//! Lifecycle handlers against real temporary git repositories: create,
//! lock, status, claim/release, remove guards, validate, and the prune
//! sweep (including reconcile-after-manual-removal).

mod support;

use std::path::Path;

use serde_json::json;
use support::{commit_file, git, head_sha, init_repo, make_env, test_config, TestEnv};
use worktree::error::codes;
use worktree::functions::{claim, create, list, prune, release, remove, status, validate};
use worktree::state;
use worktree::types::Lifecycle;

async fn create_wt(env: &TestEnv, repo: &Path, session: Option<&str>) -> create::Response {
    create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: None,
            pr: None,
            session_id: session.map(str::to_string),
        },
    )
    .await
    .expect("create worktree")
}

#[tokio::test]
async fn create_locks_registers_and_emits() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let resp = create_wt(&env, &repo, None).await;
    assert!(resp.worktree_id.starts_with("wt_"));
    assert_eq!(resp.branch, format!("iii/{}", resp.worktree_id));
    assert_eq!(resp.base_ref, "HEAD");
    assert_eq!(resp.base_sha, head_sha(&repo));
    assert!(Path::new(&resp.path).is_dir());

    let porcelain = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(
        porcelain.contains(&format!("locked iii:worktree {}", resp.worktree_id)),
        "expected lock reason in: {porcelain}"
    );

    let record = state::get_record(env.state.as_ref(), &resp.worktree_id)
        .await
        .unwrap()
        .expect("record persisted");
    assert_eq!(record.lifecycle, Lifecycle::Active);
    assert_eq!(record.path, resp.path);

    let created = env.deliverer.events_of("worktree::created");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["worktree_id"], resp.worktree_id.as_str());
}

#[tokio::test]
async fn create_rejects_existing_branch_and_bad_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    git(&repo, &["branch", "taken"]);
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let err = create::handle(
        &env.deps,
        create::Request {
            repo_path: repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: Some("taken".into()),
            pr: None,
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::BRANCH_EXISTS);

    let err = create::handle(
        &env.deps,
        create::Request {
            repo_path: "relative/repo".into(),
            base_ref: None,
            branch: None,
            pr: None,
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::BAD_PATH);

    let not_repo = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&not_repo).unwrap();
    let err = create::handle(
        &env.deps,
        create::Request {
            repo_path: not_repo.to_string_lossy().into_owned(),
            base_ref: None,
            branch: None,
            pr: None,
            session_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::NOT_A_REPO);
}

#[tokio::test]
async fn status_tracks_dirt_and_committed_work() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let resp = create_wt(&env, &repo, None).await;
    let wt = Path::new(&resp.path);

    let st = status::handle(
        &env.deps,
        status::Request {
            worktree_id: resp.worktree_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(st.status.clean);
    assert_eq!(st.status.ahead, 0);
    assert!(!st.status.in_rebase);
    assert_eq!(st.status.head_sha, resp.base_sha);

    std::fs::write(wt.join("scratch.txt"), "wip\n").unwrap();
    let st = status::handle(
        &env.deps,
        status::Request {
            worktree_id: resp.worktree_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!st.status.clean);
    assert_eq!(st.status.untracked, 1);

    commit_file(wt, "scratch.txt", "wip\n", "work");
    let st = status::handle(
        &env.deps,
        status::Request {
            worktree_id: resp.worktree_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(st.status.clean);
    assert_eq!(st.status.ahead, 1);
    assert_eq!(st.status.unpushed, 1);
    assert!(st.status.diffstat.contains("1 file changed"));
}

#[tokio::test]
async fn claim_and_release_enforce_ownership() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let resp = create_wt(&env, &repo, None).await;
    let id = resp.worktree_id.clone();

    let claimed = claim::handle(
        &env.deps,
        claim::Request {
            worktree_id: id.clone(),
            session_id: "s_1".into(),
            force: false,
        },
    )
    .await
    .unwrap();
    assert!(claimed.claimed);
    assert_eq!(claimed.previous_session_id, None);

    let err = claim::handle(
        &env.deps,
        claim::Request {
            worktree_id: id.clone(),
            session_id: "s_2".into(),
            force: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::ALREADY_CLAIMED);

    let taken = claim::handle(
        &env.deps,
        claim::Request {
            worktree_id: id.clone(),
            session_id: "s_2".into(),
            force: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(taken.previous_session_id.as_deref(), Some("s_1"));

    let err = release::handle(
        &env.deps,
        release::Request {
            worktree_id: id.clone(),
            session_id: "s_1".into(),
            force: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::CLAIM_MISMATCH);

    let released = release::handle(
        &env.deps,
        release::Request {
            worktree_id: id.clone(),
            session_id: "s_2".into(),
            force: false,
        },
    )
    .await
    .unwrap();
    assert!(released.released);
    let record = state::get_record(env.state.as_ref(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.lifecycle, Lifecycle::Active);
    assert_eq!(record.session_id, None);
    assert_eq!(env.deliverer.events_of("worktree::claimed").len(), 2);
    assert_eq!(env.deliverer.events_of("worktree::released").len(), 1);
}

#[tokio::test]
async fn remove_guards_dirty_and_unmerged_work() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let resp = create_wt(&env, &repo, None).await;
    let wt = Path::new(&resp.path).to_path_buf();

    std::fs::write(wt.join("wip.txt"), "dirty\n").unwrap();
    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: resp.worktree_id.clone(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::DIRTY);

    commit_file(&wt, "wip.txt", "dirty\n", "unmerged work");
    let err = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: resp.worktree_id.clone(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, codes::UNMERGED_WORK);

    let removed = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: resp.worktree_id.clone(),
            force: true,
            delete_branch: true,
        },
    )
    .await
    .unwrap();
    assert!(removed.removed);
    assert!(removed.branch_deleted);
    assert!(!wt.exists());
    assert!(state::get_record(env.state.as_ref(), &resp.worktree_id)
        .await
        .unwrap()
        .is_none());
    let removed_events = env.deliverer.events_of("worktree::removed");
    assert_eq!(removed_events.len(), 1);
    assert_eq!(removed_events[0]["branch_deleted"], true);
}

#[tokio::test]
async fn remove_clean_needs_no_force() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let resp = create_wt(&env, &repo, None).await;
    let removed = remove::handle(
        &env.deps,
        remove::Request {
            worktree_id: resp.worktree_id.clone(),
            force: false,
            delete_branch: false,
        },
    )
    .await
    .unwrap();
    assert!(removed.removed);
    assert!(!Path::new(&resp.path).exists());
}

#[tokio::test]
async fn validate_distinguishes_managed_unmanaged_orphaned() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));
    let resp = create_wt(&env, &repo, None).await;

    let ok = validate::handle(
        &env.deps,
        validate::Request {
            path: resp.path.clone(),
        },
    )
    .await
    .unwrap();
    assert!(ok.valid);
    assert!(ok.managed);
    assert_eq!(ok.worktree_id.as_deref(), Some(resp.worktree_id.as_str()));

    // A worktree created out of band is reported but never adopted.
    let manual = tmp.path().join("manual-wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "manual-branch",
            manual.to_str().unwrap(),
        ],
    );
    let unmanaged = validate::handle(
        &env.deps,
        validate::Request {
            path: manual.to_string_lossy().into_owned(),
        },
    )
    .await
    .unwrap();
    assert!(!unmanaged.valid);
    assert!(!unmanaged.managed);
    assert!(unmanaged.reason.unwrap().contains("unmanaged"));

    let listed = list::handle(
        &env.deps,
        list::Request {
            repo_path: Some(repo.to_string_lossy().into_owned()),
            session_id: None,
            include_status: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.worktrees.len(), 1);
    assert!(listed.worktrees[0].status.as_ref().unwrap().clean);
    assert_eq!(listed.unmanaged.len(), 1);
    // git reports canonical paths (symlinks resolved).
    let manual_canonical = std::fs::canonicalize(&manual).unwrap();
    assert_eq!(listed.unmanaged[0].path, manual_canonical.to_string_lossy());

    // Manual removal orphans the record; validate reconciles it.
    std::fs::remove_dir_all(&resp.path).unwrap();
    let orphaned = validate::handle(
        &env.deps,
        validate::Request {
            path: resp.path.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!orphaned.valid);
    assert!(orphaned.managed);
    assert!(orphaned.reason.unwrap().contains("orphaned"));
    let record = state::get_record(env.state.as_ref(), &resp.worktree_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.lifecycle, Lifecycle::Orphaned);
}

#[tokio::test]
async fn prune_reconciles_after_manual_removal_and_skips_claimed() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let env = make_env(tmp.path(), test_config(tmp.path()));

    let gone = create_wt(&env, &repo, None).await;
    let claimed = create_wt(&env, &repo, Some("s_1")).await;
    let fresh = create_wt(&env, &repo, None).await;
    std::fs::remove_dir_all(&gone.path).unwrap();

    let out = prune::handle(
        &env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.pruned, vec![gone.worktree_id.clone()]);
    let reasons: Vec<(String, String)> =
        out.skipped.into_iter().map(|s| (s.id, s.reason)).collect();
    assert!(reasons.contains(&(claimed.worktree_id.clone(), "claimed".into())));
    assert!(reasons.contains(&(fresh.worktree_id.clone(), "not expired".into())));

    assert!(state::get_record(env.state.as_ref(), &gone.worktree_id)
        .await
        .unwrap()
        .is_none());
    // The stale admin entry is pruned from git too.
    let porcelain = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(!porcelain.contains(&gone.worktree_id));
}

#[tokio::test]
async fn prune_expires_clean_worktrees_and_keeps_work() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let mut cfg = test_config(tmp.path());
    cfg.prune_expire_hours = 0;
    let env = make_env(tmp.path(), cfg);

    let expired = create_wt(&env, &repo, None).await;
    let dirty = create_wt(&env, &repo, None).await;
    std::fs::write(Path::new(&dirty.path).join("wip.txt"), "wip\n").unwrap();
    let has_work = create_wt(&env, &repo, None).await;
    commit_file(Path::new(&has_work.path), "work.txt", "w\n", "unlanded");

    let dry = prune::handle(
        &env.deps,
        prune::Request {
            repo_path: Some(repo.to_string_lossy().into_owned()),
            dry_run: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(dry.pruned, vec![expired.worktree_id.clone()]);
    assert!(Path::new(&expired.path).is_dir(), "dry_run must not act");

    let out = prune::handle(
        &env.deps,
        prune::Request {
            repo_path: Some(repo.to_string_lossy().into_owned()),
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.pruned, vec![expired.worktree_id.clone()]);
    let reasons: Vec<(String, String)> =
        out.skipped.into_iter().map(|s| (s.id, s.reason)).collect();
    assert!(reasons.contains(&(dirty.worktree_id.clone(), "dirty".into())));
    assert!(reasons.contains(&(has_work.worktree_id.clone(), "unmerged work".into())));

    assert!(!Path::new(&expired.path).exists());
    assert!(Path::new(&dirty.path).is_dir());
    assert!(Path::new(&has_work.path).is_dir());
}

#[test]
fn prune_request_accepts_empty_cron_payload() {
    let req: prune::Request = serde_json::from_value(json!({})).unwrap();
    assert!(req.repo_path.is_none());
    assert!(!req.dry_run);
}

#[tokio::test]
async fn cas_branch_delete_refuses_when_the_branch_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let first = head_sha(&repo);
    git(&repo, &["branch", "victim"]);
    // The branch moves past the sha the caller expects.
    let moved = commit_file(&repo, "move.txt", "m\n", "moves victim");
    git(&repo, &["update-ref", "refs/heads/victim", &moved]);

    let refused = worktree::git::ops::cas_branch_delete(&repo, "victim", &first, 30_000)
        .await
        .unwrap();
    assert!(!refused, "delete must refuse when the branch moved");
    let branches = git(&repo, &["branch", "--list", "victim"]);
    assert!(branches.contains("victim"), "branch retained");

    let deleted = worktree::git::ops::cas_branch_delete(&repo, "victim", &moved, 30_000)
        .await
        .unwrap();
    assert!(deleted);
    let branches = git(&repo, &["branch", "--list", "victim"]);
    assert!(!branches.contains("victim"));
}
