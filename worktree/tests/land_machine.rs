//! The land phase machine against real temporary repositories with an
//! in-memory store and a scripted test gate: happy fast-forward (CAS and
//! checked-out-primary paths), rebase conflicts left in progress, test
//! failures, target movement (bounded CAS retry), dirty-primary blocking,
//! and redelivery at every phase boundary.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use support::{
    branch_sha, commit_file, create_request, git, head_sha, init_repo, make_env,
    make_env_with_runner, test_config, FailingStore, ScriptedTestRunner, TestEnv,
};
use worktree::functions::{create, land, prune};
use worktree::land::{run_step, LandDeps};
use worktree::state;
use worktree::types::{LandPhase, Lifecycle};

struct Scenario {
    env: TestEnv,
    repo: PathBuf,
    worktree_id: String,
    wt_path: PathBuf,
}

/// Repo with `main` (checked out in the primary) and `trunk` (unchecked),
/// plus one managed worktree carrying one commit of work.
async fn setup(env: TestEnv, tmp: &Path) -> Scenario {
    let repo = tmp.join("repo");
    init_repo(&repo);
    git(&repo, &["branch", "trunk"]);
    let created = create::handle(&env.deps, create_request(&repo))
        .await
        .unwrap();
    let wt_path = PathBuf::from(&created.path);
    commit_file(&wt_path, "feature.txt", "feature\n", "add feature");
    Scenario {
        env,
        repo,
        worktree_id: created.worktree_id,
        wt_path,
    }
}

async fn enqueue_land(s: &Scenario, target: &str, req_overrides: LandOverrides) -> String {
    let resp = land::handle(
        &s.env.deps,
        land::Request {
            worktree_id: s.worktree_id.clone(),
            target_branch: target.to_string(),
            test_cmd: req_overrides.test_cmd,
            force_restart: req_overrides.force_restart,
            keep: req_overrides.keep,
        },
    )
    .await
    .unwrap();
    assert!(resp.queued);
    resp.job_id
}

#[derive(Default)]
struct LandOverrides {
    test_cmd: Option<String>,
    force_restart: bool,
    keep: bool,
}

async fn land_deps(env: &TestEnv) -> LandDeps {
    let cfg = env.deps.cfg().await;
    env.deps.land_deps(&cfg)
}

#[tokio::test]
async fn happy_land_via_cas_to_unchecked_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    let feature_sha = head_sha(&s.wt_path);

    let job_id = enqueue_land(&s, "trunk", LandOverrides::default()).await;
    let (queue, function_id, payload) = s.env.enqueuer.last().unwrap();
    assert_eq!(queue, "worktree-land");
    assert_eq!(function_id, "worktree::land-step");
    assert_eq!(payload["job_id"], job_id.as_str());
    assert!(payload["repo_key"].as_str().unwrap().ends_with(".git"));

    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);
    assert_eq!(result.phase_completed, LandPhase::Finalize);

    assert_eq!(branch_sha(&s.repo, "trunk"), feature_sha);
    assert!(!s.wt_path.exists(), "worktree removed after land");
    assert!(state::get_record(s.env.state.as_ref(), &s.worktree_id)
        .await
        .unwrap()
        .is_none());
    let porcelain = git(&s.repo, &["worktree", "list", "--porcelain"]);
    assert!(!porcelain.contains(&s.worktree_id));
    let branches = git(&s.repo, &["branch", "--list"]);
    assert!(!branches.contains(&format!("iii/{}", s.worktree_id)));

    let landed = s.env.deliverer.events_of("worktree::landed");
    assert_eq!(landed.len(), 1);
    assert_eq!(landed[0]["merged_sha"], feature_sha.as_str());
    let job = state::get_job(s.env.state.as_ref(), &job_id)
        .await
        .unwrap()
        .unwrap();
    assert!(job.done);
    assert_eq!(job.phase, LandPhase::Done);
    assert_eq!(job.blocked_code, None);
}

#[tokio::test]
async fn happy_land_into_clean_checked_out_primary() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    let feature_sha = head_sha(&s.wt_path);

    let job_id = enqueue_land(&s, "main", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    run_step(&deps, &job_id).await.unwrap();

    assert_eq!(branch_sha(&s.repo, "main"), feature_sha);
    // The primary checkout's tree was fast-forwarded too, never desynced.
    assert!(s.repo.join("feature.txt").is_file());
    assert_eq!(s.env.deliverer.events_of("worktree::landed").len(), 1);
}

#[tokio::test]
async fn dirty_checked_out_primary_blocks_with_w413() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    std::fs::write(s.repo.join("README.md"), "local edits\n").unwrap();
    let main_before = branch_sha(&s.repo, "main");

    let job_id = enqueue_land(&s, "main", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);

    assert_eq!(branch_sha(&s.repo, "main"), main_before, "main untouched");
    let blocked = s.env.deliverer.events_of("worktree::land-blocked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["code"], "W413");
    assert_eq!(blocked[0]["reason"], "target_checked_out_dirty");
    let record = state::get_record(s.env.state.as_ref(), &s.worktree_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.lifecycle, Lifecycle::LandBlocked);
    assert!(s.wt_path.is_dir(), "worktree kept for another attempt");
    // A blocked land releases the git lock; the worktree is no longer locked.
    let porcelain = git(&s.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        !porcelain.contains(&format!("locked iii:worktree {}", s.worktree_id)),
        "blocked worktree must be unlocked, got:\n{porcelain}"
    );
}

#[tokio::test]
async fn prune_preserves_blocked_with_unmerged_then_reaps_integrated() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = test_config(tmp.path());
    cfg.prune_expire_hours = 0; // everything is age-eligible
    let s = setup(make_env(tmp.path(), cfg), tmp.path()).await;
    let feature_sha = head_sha(&s.wt_path);

    // Reach LandBlocked via W413 (dirty primary): the worktree itself stays
    // clean, one commit ahead of main, and main is untouched (not integrated).
    std::fs::write(s.repo.join("README.md"), "local edits\n").unwrap();
    let job_id = enqueue_land(&s, "main", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    run_step(&deps, &job_id).await.unwrap();
    let rec = state::get_record(s.env.state.as_ref(), &s.worktree_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec.lifecycle, Lifecycle::LandBlocked);

    // Unmerged work must be preserved, not reaped.
    let out = prune::handle(
        &s.env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert!(
        !out.pruned.contains(&s.worktree_id),
        "must not prune unmerged: {out:?}"
    );
    assert!(out
        .skipped
        .iter()
        .any(|k| k.id == s.worktree_id && k.reason == "unmerged work"));
    assert!(s.wt_path.is_dir());

    // Once the work is integrated, prune reaps it.
    let _ = git(&s.repo, &["merge", "--ff-only", &feature_sha]);
    let out2 = prune::handle(
        &s.env.deps,
        prune::Request {
            repo_path: None,
            dry_run: false,
        },
    )
    .await
    .unwrap();
    assert!(
        out2.pruned.contains(&s.worktree_id),
        "must reap integrated: {out2:?}"
    );
}

#[tokio::test]
async fn rebase_conflict_left_in_progress_then_w402_then_force_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    // Conflicting edits: the worktree and main both rewrite README.md.
    commit_file(&s.wt_path, "README.md", "worktree version\n", "wt edit");
    commit_file(&s.repo, "README.md", "main version\n", "main edit");

    let job_id = enqueue_land(&s, "main", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);
    assert_eq!(result.phase_completed, LandPhase::Rebase);

    let blocked = s.env.deliverer.events_of("worktree::land-blocked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["code"], "W410");
    assert_eq!(blocked[0]["reason"], "rebase_conflict");
    assert_eq!(blocked[0]["conflict_files"][0], "README.md");

    // The rebase is left in progress inside the worktree for in-place
    // resolution by the owning agent.
    let wt_status = git(&s.wt_path, &["status"]);
    assert!(
        wt_status.contains("rebase in progress"),
        "expected in-progress rebase, got: {wt_status}"
    );

    // A new land without force_restart refuses with W402.
    let err = land::handle(
        &s.env.deps,
        land::Request {
            worktree_id: s.worktree_id.clone(),
            target_branch: "main".into(),
            test_cmd: None,
            force_restart: false,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W402");

    // With force_restart the conflicted rebase is aborted and a fresh job
    // queues (it will conflict again here; queueing is what we assert).
    let retry = land::handle(
        &s.env.deps,
        land::Request {
            worktree_id: s.worktree_id.clone(),
            target_branch: "main".into(),
            test_cmd: None,
            force_restart: true,
            keep: false,
        },
    )
    .await
    .unwrap();
    assert!(retry.queued);
    assert_ne!(retry.job_id, job_id);
}

#[tokio::test]
async fn preflight_dirty_worktree_blocks_with_w220() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    std::fs::write(s.wt_path.join("uncommitted.txt"), "wip\n").unwrap();

    let job_id = enqueue_land(&s, "trunk", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);
    let blocked = s.env.deliverer.events_of("worktree::land-blocked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["code"], "W220");
    assert_eq!(blocked[0]["reason"], "dirty");
}

#[tokio::test]
async fn test_gate_failure_blocks_with_w411_and_keeps_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let runner = Arc::new(ScriptedTestRunner::new(|_| 1));
    let env = make_env_with_runner(tmp.path(), test_config(tmp.path()), runner.clone());
    let s = setup(env, tmp.path()).await;
    let trunk_before = branch_sha(&s.repo, "trunk");

    let job_id = enqueue_land(
        &s,
        "trunk",
        LandOverrides {
            test_cmd: Some("exit 1".into()),
            ..Default::default()
        },
    )
    .await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);
    assert_eq!(result.phase_completed, LandPhase::Test);
    assert_eq!(runner.call_count(), 1);

    assert_eq!(branch_sha(&s.repo, "trunk"), trunk_before);
    assert!(s.wt_path.is_dir());
    let blocked = s.env.deliverer.events_of("worktree::land-blocked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["code"], "W411");
    assert_eq!(blocked[0]["reason"], "test_failed");
    assert_eq!(blocked[0]["exit_code"], 1);
}

/// Advance `trunk` out from under the land by committing on main (in the
/// primary) and pointing trunk at it.
fn advance_trunk(repo: &Path, n: u32) {
    commit_file(
        repo,
        &format!("racer-{n}.txt"),
        "racing\n",
        &format!("racer {n}"),
    );
    let sha = head_sha(repo);
    git(repo, &["update-ref", "refs/heads/trunk", &sha]);
}

#[tokio::test]
async fn target_moved_once_cas_retry_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_for_script = tmp.path().join("repo");
    let script_repo = repo_for_script.clone();
    // First test-gate call advances the target; the retry pass leaves it be.
    let runner = Arc::new(ScriptedTestRunner::new(move |n| {
        if n == 0 {
            advance_trunk(&script_repo, n);
        }
        0
    }));
    let env = make_env_with_runner(tmp.path(), test_config(tmp.path()), runner.clone());
    let s = setup(env, tmp.path()).await;

    let job_id = enqueue_land(
        &s,
        "trunk",
        LandOverrides {
            test_cmd: Some("true".into()),
            ..Default::default()
        },
    )
    .await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);

    assert_eq!(
        runner.call_count(),
        2,
        "test gate reruns after the retry rebase"
    );
    let job = state::get_job(s.env.state.as_ref(), &job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.cas_attempts, 1);
    assert_eq!(job.blocked_code, None);
    // The landed head carries both the racer commit and the feature.
    assert_eq!(branch_sha(&s.repo, "trunk"), job.merged_sha.unwrap());
    assert_eq!(s.env.deliverer.events_of("worktree::landed").len(), 1);
}

#[tokio::test]
async fn target_moved_past_retry_bound_blocks_with_w412() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_for_script = tmp.path().join("repo");
    let script_repo = repo_for_script.clone();
    // Every test-gate pass advances the target: the CAS can never win.
    let runner = Arc::new(ScriptedTestRunner::new(move |n| {
        advance_trunk(&script_repo, n);
        0
    }));
    let mut cfg = test_config(tmp.path());
    cfg.max_land_retries = 1;
    let env = make_env_with_runner(tmp.path(), cfg, runner.clone());
    let s = setup(env, tmp.path()).await;

    let job_id = enqueue_land(
        &s,
        "trunk",
        LandOverrides {
            test_cmd: Some("true".into()),
            ..Default::default()
        },
    )
    .await;
    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, &job_id).await.unwrap();
    assert!(result.done);

    assert_eq!(runner.call_count(), 2);
    let blocked = s.env.deliverer.events_of("worktree::land-blocked");
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0]["code"], "W412");
    assert_eq!(blocked[0]["reason"], "target_moved_exhausted");
    let job = state::get_job(s.env.state.as_ref(), &job_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.cas_attempts, 2);
}

#[tokio::test]
async fn keep_preserves_worktree_and_rebases_record_base() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    let feature_sha = head_sha(&s.wt_path);

    let job_id = enqueue_land(
        &s,
        "trunk",
        LandOverrides {
            keep: true,
            ..Default::default()
        },
    )
    .await;
    let deps = land_deps(&s.env).await;
    run_step(&deps, &job_id).await.unwrap();

    assert_eq!(branch_sha(&s.repo, "trunk"), feature_sha);
    assert!(s.wt_path.is_dir(), "keep preserves the worktree");
    let record = state::get_record(s.env.state.as_ref(), &s.worktree_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.lifecycle, Lifecycle::Active);
    assert_eq!(record.base_ref, "trunk");
    assert_eq!(record.base_sha, feature_sha);
}

#[tokio::test]
async fn redelivery_at_every_phase_boundary_converges() {
    // Measure the happy path's write count once, then crash-inject before
    // each write in turn: run_step errors (infra), the redelivery re-enters
    // at the persisted phase and converges with exactly one merge.
    let total_writes = {
        let tmp = tempfile::tempdir().unwrap();
        let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
        let job_id = enqueue_land(&s, "trunk", LandOverrides::default()).await;
        let counting = Arc::new(FailingStore::new(s.env.state.clone(), 0));
        let mut deps = land_deps(&s.env).await;
        deps.state = counting.clone();
        run_step(&deps, &job_id).await.unwrap();
        counting.set_count()
    };
    assert!(
        total_writes >= 4,
        "expected one persisted transition per phase, got {total_writes}"
    );

    for fail_at in 1..=total_writes {
        let tmp = tempfile::tempdir().unwrap();
        let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
        let feature_sha = head_sha(&s.wt_path);
        let job_id = enqueue_land(&s, "trunk", LandOverrides::default()).await;

        let failing = Arc::new(FailingStore::new(s.env.state.clone(), fail_at));
        let mut deps = land_deps(&s.env).await;
        deps.state = failing.clone();

        let err = run_step(&deps, &job_id)
            .await
            .expect_err("injected crash must surface as an infra error");
        assert_eq!(err.code, "W300", "crash at write {fail_at}");
        assert!(failing.tripped());

        // Queue redelivery: same job id, healed store.
        let result = run_step(&deps, &job_id)
            .await
            .unwrap_or_else(|e| panic!("redelivery after crash at write {fail_at}: {e}"));
        assert!(result.done);

        assert_eq!(
            branch_sha(&s.repo, "trunk"),
            feature_sha,
            "exactly one merge after crash at write {fail_at}"
        );
        let job = state::get_job(s.env.state.as_ref(), &job_id)
            .await
            .unwrap()
            .unwrap();
        assert!(job.done, "job terminal after crash at write {fail_at}");
        assert_eq!(job.blocked_code, None);
        assert!(
            !s.env.deliverer.events_of("worktree::landed").is_empty(),
            "landed emitted at least once after crash at write {fail_at}"
        );

        // A further redelivery after completion is a no-op.
        let landed_before = s.env.deliverer.count();
        let again = run_step(&deps, &job_id).await.unwrap();
        assert!(again.done);
        assert_eq!(
            s.env.deliverer.count(),
            landed_before,
            "no duplicate events"
        );
    }
}

#[tokio::test]
async fn backup_ref_survives_a_blocked_land_and_clears_on_success() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    // Conflicting edits so the first land blocks in the rebase phase.
    commit_file(&s.wt_path, "README.md", "worktree version\n", "wt edit");
    commit_file(&s.repo, "README.md", "main version\n", "main edit");
    let blocked_head = head_sha(&s.wt_path);

    let job_id = enqueue_land(&s, "main", LandOverrides::default()).await;
    let deps = land_deps(&s.env).await;
    run_step(&deps, &job_id).await.unwrap();

    let backup = format!("refs/iii/wt-backup/{}", s.worktree_id);
    let pinned = git(&s.repo, &["rev-parse", &backup]);
    assert_eq!(pinned, blocked_head, "blocked land keeps the backup ref");

    // Resolve by force-restarting against trunk (no conflict there); the
    // successful finalize releases the anchor.
    let retry = land::handle(
        &s.env.deps,
        land::Request {
            worktree_id: s.worktree_id.clone(),
            target_branch: "trunk".into(),
            test_cmd: None,
            force_restart: true,
            keep: false,
        },
    )
    .await
    .unwrap();
    run_step(&deps, &retry.job_id).await.unwrap();

    let out = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &backup])
        .current_dir(&s.repo)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "successful finalize deletes the backup ref"
    );
}

#[tokio::test]
async fn second_land_while_job_live_is_w401_and_unknown_job_is_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let s = setup(make_env(tmp.path(), test_config(tmp.path())), tmp.path()).await;
    let _job_id = enqueue_land(&s, "trunk", LandOverrides::default()).await;

    let err = land::handle(
        &s.env.deps,
        land::Request {
            worktree_id: s.worktree_id.clone(),
            target_branch: "trunk".into(),
            test_cmd: None,
            force_restart: false,
            keep: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "W401");

    let deps = land_deps(&s.env).await;
    let result = run_step(&deps, "job_nonexistent").await.unwrap();
    assert!(result.done);
}
