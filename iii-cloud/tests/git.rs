//! `project::resolve` and `project::use` on real temp repositories.

mod support;

use iii_cloud::check::pushed;
use iii_cloud::project::{resolve, use_project};
use iii_cloud::types::*;
use support::repo::*;
use wiremock::MockServer;

fn use_req(path: &std::path::Path, bring: bool) -> UseRequest {
    UseRequest {
        cloud_project_id: None,
        path: path.to_string_lossy().into(),
        environment_id: Some(ENV.into()),
        mode: WorkspaceMode::Worktree,
        bring_changes: Some(bring),
        from: None,
        source: None,
    }
}

#[tokio::test]
async fn resolve_reads_the_raw_remote_and_the_checkout() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    std::fs::write(r.checkout.join("new.txt"), "x").unwrap();
    let p = resolve(
        &h.ctx,
        ResolveRequest {
            path: r.checkout.to_string_lossy().into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap();
    // The insteadOf rule does not hide the GitHub name.
    assert_eq!(p.repository, "ada/hello-iii");
    assert_eq!(p.checkout.branch.as_deref(), Some("main"));
    assert_eq!(p.checkout.changed_files, vec!["new.txt"]);
    assert_eq!(p.checkout.ahead, 0);
    assert_eq!(p.environment.as_ref().unwrap().id, ENV);
    assert!(
        p.cloud_project.is_none() && p.github_access.is_none(),
        "a key can't match the repository"
    );

    let err = resolve(
        &h.ctx,
        ResolveRequest {
            path: r.root.path().to_string_lossy().into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NOT_A_GIT_REPO");
    let err = resolve(
        &h.ctx,
        ResolveRequest {
            path: "/nonexistent/x".into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NOT_A_GIT_REPO");
    git(
        &r.checkout,
        &["remote", "set-url", "origin", "https://gitlab.com/a/b.git"],
    );
    let err = resolve(
        &h.ctx,
        ResolveRequest {
            path: r.checkout.to_string_lossy().into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "NO_GITHUB_REMOTE");
}

#[tokio::test]
async fn use_makes_a_worktree_from_origin_and_leaves_the_checkout_alone() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    // A local-only commit and a dirty file: neither may reach the worktree or change.
    std::fs::write(r.checkout.join("dirty.txt"), "x").unwrap();
    let (head, status) = (
        git(&r.checkout, &["rev-parse", "HEAD"]),
        git(&r.checkout, &["status", "--porcelain"]),
    );
    let p = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let ws = p.workspace.clone().unwrap();
    assert_eq!(ws.mode, WorkspaceMode::Worktree);
    assert_eq!(ws.branch, "iii-cloud/ready-production");
    assert_eq!(
        std::path::PathBuf::from(&ws.path),
        r.checkout.parent().unwrap().join("hello-iii-iii-cloud")
    );
    assert_eq!(
        git(std::path::Path::new(&ws.path), &["rev-parse", "HEAD"]),
        git(&r.origin, &["rev-parse", "main"])
    );
    assert!(!std::path::Path::new(&ws.path).join("dirty.txt").exists());
    assert_eq!(
        (
            git(&r.checkout, &["rev-parse", "HEAD"]),
            git(&r.checkout, &["status", "--porcelain"])
        ),
        (head, status)
    );
    assert_eq!(p.environment.unwrap().branch, "main");
    // The run's own files are hidden in the shared exclude.
    let exclude = std::fs::read_to_string(r.checkout.join(".git/info/exclude")).unwrap();
    for pattern in [
        ".iii/",
        "*.iii-cloud-run.yaml",
        "*.iii-cloud-run.lock",
        ".env",
    ] {
        assert!(exclude.lines().any(|l| l == pattern), "{pattern}");
    }
    assert!(!exclude.contains("/.iii/"));
    // The store knows the workspace, and the project event was sent.
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.project.as_ref().unwrap().workspace.clone()),
        Some(ws.clone())
    );
    assert!(h.fake.kinds().iter().any(|k| k == "project:true"));
    // Again: the same worktree is reused.
    let again = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    assert_eq!(again.workspace.unwrap(), ws);
    // Excludes are not duplicated.
    let exclude = std::fs::read_to_string(r.checkout.join(".git/info/exclude")).unwrap();
    assert_eq!(exclude.lines().filter(|l| *l == ".iii/").count(), 1);
}

#[tokio::test]
async fn a_path_taken_by_something_else_is_refused() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    std::fs::create_dir_all(r.checkout.parent().unwrap().join("hello-iii-iii-cloud")).unwrap();
    let err = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "WORKTREE_PATH_TAKEN");
}

/// `iii-cloud/ready-production` pushed with the agent's `commits` on top of
/// main (from a clone, so the checkout knows it only from origin).
fn pushed_fix(r: &Repo, files: &[&str]) -> String {
    let t = r.root.path().join("agent");
    git(
        r.root.path(),
        &["clone", "-q", r.origin.to_str().unwrap(), "agent"],
    );
    git(&t, &["checkout", "-q", "-b", "iii-cloud/ready-production"]);
    for f in files {
        std::fs::write(t.join(f), f).unwrap();
        git(&t, &["add", "."]);
        git(&t, &["commit", "-q", "-m", &format!("Add {f}")]);
    }
    git(&t, &["push", "-q", "origin", "iii-cloud/ready-production"]);
    git(&t, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn an_earlier_fix_branch_is_resumed_with_its_commits_and_the_base_brought_in() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    // The agent's two commits on origin; the local branch has only the first.
    let pushed = pushed_fix(&r, &["one.txt", "two.txt"]);
    git(&r.checkout, &["fetch", "-q", "origin"]);
    git(
        &r.checkout,
        &[
            "branch",
            "iii-cloud/ready-production",
            &format!("{pushed}~1"),
        ],
    );
    // main moved since.
    let moved = move_main(&r, "m.txt", "m");
    let p = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let w = p.workspace.unwrap();
    assert_eq!(
        w.branch, "iii-cloud/ready-production",
        "the same name, not -2"
    );
    let ws = std::path::PathBuf::from(&w.path);
    for f in ["one.txt", "two.txt", "m.txt"] {
        assert!(ws.join(f).exists(), "{f}");
    }
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%P"]),
        format!("{pushed} {moved}"),
        "origin's commits, then the base merged in"
    );
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%s"]),
        "Merge main into iii-cloud/ready-production"
    );
    assert_eq!(
        git(&r.origin, &["rev-parse", "iii-cloud/ready-production"]),
        pushed,
        "nothing pushed"
    );
    // The agent's commits are not an earlier try's.
    assert!(
        h.ctx
            .store
            .read(|s| s.worktree_started.keys().all(|k| !k.starts_with(&w.path)))
    );
}

/// The page's "moved since" compares the source's tip with `source_sha`,
/// never the worktree's own head: brought changes and the person's later
/// commits on another branch don't make it differ.
#[tokio::test]
async fn source_sha_is_what_the_source_pointed_at_when_it_was_taken() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    std::fs::write(r.checkout.join("new.txt"), "new").unwrap();
    let req = UseRequest {
        source: Some("origin/main".into()),
        ..from_checkout(&r.checkout, ENV2, true)
    };
    let w = use_project(&h.ctx, req).await.unwrap().workspace.unwrap();
    let main = git(&r.checkout, &["rev-parse", "origin/main"]);
    assert_eq!(w.source_sha.as_deref(), Some(main.as_str()));
    assert_ne!(w.from_head, w.source_sha, "the brought commit is on top");
    // The person commits on their own branch: the source did not move.
    old_commit(&r.checkout, "later.txt", "later");
    let plain = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some(ENV2.into()),
            ..use_req(&r.checkout, false)
        },
    )
    .await
    .unwrap()
    .workspace
    .unwrap();
    assert_eq!(plain.source_sha.as_deref(), Some(main.as_str()));
    // Without a source it is the checkout's HEAD.
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        w.source_sha.as_deref(),
        Some(git(&r.checkout, &["rev-parse", "HEAD"]).as_str())
    );
}

#[tokio::test]
async fn a_resumed_fix_branch_that_conflicts_with_the_base_is_left_as_it_was() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    let pushed = pushed_fix(&r, &["README.md"]);
    move_main(&r, "README.md", "theirs");
    let err = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "MERGE_CONFLICT");
    assert!(err.message.contains("README.md"), "{}", err.message);
    let ws = r.root.path().join("work/hello-iii-iii-cloud");
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), pushed);
    assert_eq!(
        git(&ws, &["status", "--porcelain"]),
        "",
        "no merge left behind"
    );
}

#[tokio::test]
async fn a_merged_fix_branch_is_resumed_at_the_base() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    let pushed = pushed_fix(&r, &["one.txt"]);
    // Merged into main on origin, then main moved on.
    let t = r.root.path().join("agent");
    git(&t, &["checkout", "-q", "main"]);
    git(&t, &["merge", "-q", "--no-ff", "-m", "merge", &pushed]);
    std::fs::write(t.join("later.txt"), "later").unwrap();
    git(&t, &["add", "."]);
    git(&t, &["commit", "-q", "-m", "later"]);
    git(&t, &["push", "-q", "origin", "main"]);
    let main = git(&t, &["rev-parse", "HEAD"]);
    let w = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(w.branch, "iii-cloud/ready-production");
    assert_eq!(
        git(std::path::Path::new(&w.path), &["rev-parse", "HEAD"]),
        main
    );
}

#[tokio::test]
async fn bring_changes_copies_tracked_and_untracked_files() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    std::fs::write(r.checkout.join("README.md"), "hello\nmore\n").unwrap();
    std::fs::create_dir_all(r.checkout.join("src")).unwrap();
    std::fs::write(r.checkout.join("src/new.rs"), "fn main() {}\n").unwrap();
    let p = use_project(&h.ctx, use_req(&r.checkout, true))
        .await
        .unwrap();
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "hello\nmore\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("src/new.rs")).unwrap(),
        "fn main() {}\n"
    );
    // Unpushed commits are reported, not brought.
    assert_eq!(p.checkout.ahead, 0);
}

#[tokio::test]
async fn bringing_files_the_worktree_already_has_is_no_error() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    std::fs::write(r.checkout.join("new.txt"), "x\n").unwrap();
    let req = || from_checkout(&r.checkout, ENV2, true);
    let p = use_project(&h.ctx, req()).await.unwrap();
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    assert_eq!(git(&ws, &["show", "HEAD:new.txt"]), "x");
    // The same untracked file again (already committed in the worktree): nothing to commit.
    use_project(&h.ctx, req()).await.unwrap();
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), head);
}

#[tokio::test]
async fn a_patch_that_does_not_apply_leaves_the_worktree_untouched() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    let p = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    // The worktree moves README on; the checkout edits the old one, and adds another file.
    std::fs::write(ws.join("README.md"), "changed in the worktree\n").unwrap();
    git(&ws, &["commit", "-am", "wt"]);
    std::fs::write(r.checkout.join("README.md"), "changed in the checkout\n").unwrap();
    std::fs::write(r.checkout.join("other.txt"), "other\n").unwrap();
    let err = use_project(&h.ctx, use_req(&r.checkout, true))
        .await
        .unwrap_err();
    assert_eq!(err.code, "BRING_CHANGES_CONFLICT");
    assert!(err.message.contains("README.md"));
    assert!(!ws.join("other.txt").exists(), "nothing is half-applied");
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "changed in the worktree\n"
    );
}

#[tokio::test]
async fn a_commit_is_pushed_only_when_a_remote_branch_has_it() {
    let r = repo();
    git(&r.checkout, &["commit", "--allow-empty", "-m", "local"]);
    let sha = git(&r.checkout, &["rev-parse", "HEAD"]);
    assert!(!pushed(&r.checkout, &sha).await);
    git(&r.checkout, &["push", "origin", "main"]);
    assert!(pushed(&r.checkout, &sha).await);
}

const ENV2: &str = "55555555-5555-4555-8555-555555555555";

fn a_check(env: &str, commit: &str) -> Check {
    let (plan, _) =
        iii_cloud::check::plan_part(&serde_json::json!({"can_deploy": true, "plan": {}}));
    Check {
        id: format!("c-{env}"),
        environment_id: env.into(),
        commit: commit.into(),
        base: String::new(),
        checked_at: "t".into(),
        duration_ms: 0,
        plan,
        here: HerePart {
            problems: 0,
            iii_version: None,
            accepts_cloud_block: true,
        },
        problems: vec![],
        run: None,
        ready: false,
        report: String::new(),
        manifest: None,
        handoff_id: None,
        commits: vec![],
    }
}

async fn staging(server: &MockServer) {
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/teams/{TEAM}/environments/{ENV2}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": ENV2, "project_id": "p", "name": "staging", "production": false, "branch": "main", "manifest": "worker-compose.yaml", "url": null, "settings": {}})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/support-matrix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"engine": "0.27.0", "minimum_sdk": "0.26"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path_regex(r"/plan$")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))).mount(server).await;
}

#[tokio::test]
async fn switching_to_another_environment_of_the_same_folder_replaces_the_project_and_checks_again()
{
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    let first = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let first_ws = first.workspace.unwrap();
    h.ctx.store.update(|s| s.put_check(a_check(ENV, "abc")));
    let env2 = |bring| UseRequest {
        path: r.checkout.to_string_lossy().into(),
        environment_id: Some(ENV2.into()),
        mode: WorkspaceMode::Worktree,
        bring_changes: Some(bring),
        cloud_project_id: None,
        from: None,
        source: None,
    };
    let second = use_project(&h.ctx, env2(false)).await.unwrap();
    // A worktree of its own, named after the environment: the first one is not in the way.
    let ws = second.workspace.clone().unwrap();
    assert_eq!(ws.branch, "iii-cloud/ready-staging");
    assert_eq!(
        std::path::PathBuf::from(&ws.path),
        r.checkout
            .parent()
            .unwrap()
            .join("hello-iii-iii-cloud-staging")
    );
    assert_ne!(ws.path, first_ws.path);
    assert!(
        std::path::Path::new(&first_ws.path).exists(),
        "the first worktree is left alone"
    );
    assert_eq!(second.environment.as_ref().unwrap().id, ENV2);
    // The old environment's check is not the selected one's; the status shows none.
    assert!(h.ctx.store.read(|s| s.check.is_none()));
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert!(status.check.is_none() && status.project.unwrap().environment.unwrap().id == ENV2);
    // It checks again, for the new environment.
    let c = iii_cloud::check::check(&h.ctx, CheckRequest::default())
        .await
        .unwrap();
    assert_eq!(c.environment_id, ENV2);
    assert_eq!(h.ctx.store.read(|s| s.check.clone()).unwrap().id, c.id);
    // Back to the first: its own worktree is reused.
    let back = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    assert_eq!(back.workspace.unwrap().path, first_ws.path);
}

#[tokio::test]
async fn switching_to_another_folder_keeps_the_checks_and_coming_back_restores_its_check() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    use_project(&h.ctx, use_req(&r1.checkout, false))
        .await
        .unwrap();
    h.ctx.store.update(|s| s.put_check(a_check(ENV, "abc")));
    let p = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some(ENV2.into()),
            ..use_req(&r2.checkout, false)
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::path::PathBuf::from(&p.path),
        r2.checkout.canonicalize().unwrap()
    );
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    assert_eq!(
        ws,
        r2.checkout.parent().unwrap().join("hello-iii-iii-cloud")
    );
    assert!(
        h.ctx
            .store
            .read(|s| s.check.is_none() && s.checks.len() == 1),
        "staging has no check; the first folder's stays in the history"
    );
    let c = iii_cloud::check::check(&h.ctx, CheckRequest::default())
        .await
        .unwrap();
    assert_eq!(
        c.commit,
        git(&ws, &["rev-parse", "HEAD"]),
        "the check is of the new folder's worktree"
    );
    // Back to the first folder: its environment's check is the check again.
    use_project(&h.ctx, use_req(&r1.checkout, false))
        .await
        .unwrap();
    assert_eq!(h.ctx.store.read(|s| s.check.clone()).unwrap().commit, "abc");
    assert_eq!(h.ctx.store.read(|s| s.checks.len()), 2);
}

#[tokio::test]
async fn resolving_another_folder_keeps_the_selection_and_reads_it_as_the_candidate() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    let first = use_project(&h.ctx, use_req(&r1.checkout, false))
        .await
        .unwrap();
    let selected = || h.ctx.store.read(|s| s.project.clone().unwrap());
    let candidate = || h.ctx.store.read(|s| s.candidate.clone());
    let other = resolve(
        &h.ctx,
        ResolveRequest {
            path: r2.checkout.to_string_lossy().into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(selected(), first, "the selection is untouched");
    assert_eq!(candidate().unwrap().path, other.path);
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert_eq!(status.candidate.unwrap().path, other.path);
    assert_eq!(status.project.unwrap().path, first.path);
    // A `use` that fails (the cloud answers 404 for the environment) changes nothing.
    let err = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some("99999999-9999-4999-8999-999999999999".into()),
            ..use_req(&r2.checkout, false)
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "CLOUD_ERROR");
    assert_eq!(selected(), first);
    assert_eq!(candidate().unwrap().path, other.path);
    // A `use` that works selects the folder and clears the candidate.
    let p = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some(ENV2.into()),
            ..use_req(&r2.checkout, false)
        },
    )
    .await
    .unwrap();
    assert_eq!(selected().path, p.path);
    assert_eq!(p.path, other.path);
    assert_eq!(candidate(), None);
}

fn key_of(w: &Workspace) -> String {
    format!("{}\n{}", w.path, w.branch)
}

#[tokio::test]
async fn a_fresh_worktree_records_when_it_began_and_a_reused_one_keeps_it() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    let first = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let started = h
        .ctx
        .store
        .read(|s| s.worktree_started.get(&key_of(&first)).cloned())
        .expect("recorded for a fresh worktree");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let again = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.worktree_started.get(&key_of(&first)).cloned()),
        Some(started),
        "a reused worktree keeps its start"
    );
}

#[tokio::test]
async fn gone_remote_branches_are_pruned_before_the_worktree_is_made() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    // An earlier try's fix branch is on origin, and the checkout has fetched it.
    git(&r.origin, &["branch", "iii-cloud/ready-production", "main"]);
    git(&r.checkout, &["fetch", "origin"]);
    assert!(
        std::process::Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                "origin/iii-cloud/ready-production"
            ])
            .current_dir(&r.checkout)
            .status()
            .unwrap()
            .success()
    );
    // It is deleted on origin; `use` forgets it.
    git(&r.origin, &["branch", "-D", "iii-cloud/ready-production"]);
    use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    assert!(
        !std::process::Command::new("git")
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                "origin/iii-cloud/ready-production"
            ])
            .current_dir(&r.checkout)
            .status()
            .unwrap()
            .success()
    );
}

#[tokio::test]
async fn use_returns_the_branch_it_really_made_and_status_exposes_it() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    // A folder the worker did not make holds the name: the new one gets `-2`.
    let theirs = r.root.path().join("theirs");
    git(
        &r.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "iii-cloud/ready-production",
            theirs.to_str().unwrap(),
            "main",
        ],
    );
    let p = use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let w = p.workspace.unwrap();
    assert_eq!(w.branch, "iii-cloud/ready-production-2");
    assert_eq!(
        git(
            std::path::Path::new(&w.path),
            &["rev-parse", "--abbrev-ref", "HEAD"]
        ),
        w.branch
    );
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert_eq!(status.project.unwrap().workspace.unwrap().branch, w.branch);
}

/// `project::use` of staging from the person's own branch.
fn from_checkout(path: &std::path::Path, env: &str, bring: bool) -> UseRequest {
    UseRequest {
        environment_id: Some(env.into()),
        from: Some(UseFrom::Checkout),
        ..use_req(path, bring)
    }
}

/// A commit in `dir` made long ago, so only `from_head` keeps it from looking
/// like an earlier try's.
fn old_commit(dir: &std::path::Path, file: &str, message: &str) -> String {
    std::fs::write(dir.join(file), message).unwrap();
    git(dir, &["add", "."]);
    let out = std::process::Command::new("git")
        .args(["commit", "-q", "-m", message])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.test")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(dir, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn from_checkout_starts_at_the_persons_head_takes_new_commits_and_never_serves_production() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    for (k, v) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&r.checkout, &["config", k, v]);
    }
    git(&r.checkout, &["checkout", "-q", "-b", "feature"]);
    old_commit(&r.checkout, "one.txt", "mine 1");
    let mine = old_commit(&r.checkout, "two.txt", "mine 2");
    // Production goes through a pull request: never from the checkout.
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "INVALID_REQUEST");
    assert!(err.message.contains("pull request"), "{}", err.message);
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    assert_eq!(w.branch, "iii-cloud/ready-staging");
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), mine);
    assert_eq!(w.from_head.as_deref(), Some(mine.as_str()));
    // The person's commits are the base, not an earlier try.
    let c = iii_cloud::check::check(&h.ctx, CheckRequest::default())
        .await
        .unwrap();
    assert!(
        !c.problems.iter().any(|p| p.code == "OLD_ATTEMPT_COMMITS"),
        "{:?}",
        c.problems.iter().map(|p| &p.code).collect::<Vec<_>>()
    );
    // A new commit of the person's comes in by fast-forward.
    let three = old_commit(&r.checkout, "three.txt", "mine 3");
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), three);
    assert_eq!(w.from_head.as_deref(), Some(three.as_str()));
    // A plain `use` keeps what the worktree took from the checkout.
    let plain = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some(ENV2.into()),
            ..use_req(&r.checkout, false)
        },
    )
    .await
    .unwrap()
    .workspace
    .unwrap();
    assert_eq!(plain.from_head.as_deref(), Some(three.as_str()));
    // The agent committed and so did the person: the person's new commits
    // come in as a merge in the worktree.
    std::fs::write(ws.join("agent.txt"), "a").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "agent"]);
    let agent = git(&ws, &["rev-parse", "HEAD"]);
    let four = old_commit(&r.checkout, "four.txt", "mine 4");
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%P%n%s"]),
        format!("{agent} {four}\nMerge feature into main for the check")
    );
    assert_eq!(
        w.from_head.as_deref(),
        Some(git(&ws, &["rev-parse", "HEAD"]).as_str())
    );
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), four);
}

/// A commit on origin's `main` the checkout does not have.
fn move_main(r: &Repo, file: &str, text: &str) -> String {
    let t = r.root.path().join("other");
    git(
        r.root.path(),
        &["clone", "-q", r.origin.to_str().unwrap(), "other"],
    );
    std::fs::write(t.join(file), text).unwrap();
    git(&t, &["add", "."]);
    git(&t, &["commit", "-m", "moved"]);
    git(&t, &["push", "-q", "origin", "main"]);
    git(&t, &["rev-parse", "HEAD"])
}

/// The person's branch `feature`, nothing of its own yet; Ada's identity for the worker's commits.
fn feature(r: &Repo) {
    for (k, v) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&r.checkout, &["config", k, v]);
    }
    git(&r.checkout, &["checkout", "-q", "-b", "feature"]);
}

#[tokio::test]
async fn from_checkout_with_nothing_of_its_own_starts_at_the_environment_ahead_of_it() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    let mine = git(&r.checkout, &["rev-parse", "HEAD"]);
    let moved = move_main(&r, "x.txt", "x");
    std::fs::write(r.checkout.join("new.txt"), "new").unwrap();
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    assert_eq!(git(&ws, &["rev-parse", "HEAD~1"]), moved);
    assert_eq!(
        git(&ws, &["show", "--name-only", "--format=", "HEAD"]),
        "new.txt"
    );
    assert_eq!(
        w.from_head.as_deref(),
        Some(git(&ws, &["rev-parse", "HEAD"]).as_str())
    );
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), mine);
    assert_eq!(git(&r.checkout, &["status", "--porcelain"]), "?? new.txt");
}

#[tokio::test]
async fn from_checkout_diverged_merges_in_the_worktree_and_never_in_the_checkout() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    let mine = old_commit(&r.checkout, "one.txt", "mine");
    let moved = move_main(&r, "x.txt", "x");
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%P%n%s"]),
        format!("{moved} {mine}\nMerge feature into main for the check")
    );
    assert_eq!(
        w.from_head.as_deref(),
        Some(git(&ws, &["rev-parse", "HEAD"]).as_str())
    );
    assert_eq!(git(&ws, &["status", "--porcelain"]), "");
    // The checkout: same branch, same commit, nothing changed, no merge of origin's main.
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), mine);
    assert_eq!(
        git(&r.checkout, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
    assert_eq!(git(&r.checkout, &["status", "--porcelain"]), "");
    assert!(!r.checkout.join("x.txt").exists());
    // A diverging commit of the person's later is merged into the worktree that is there.
    let merged = git(&ws, &["rev-parse", "HEAD"]);
    let two = old_commit(&r.checkout, "two.txt", "mine 2");
    use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap();
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%P"]),
        format!("{merged} {two}")
    );
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), two);
}

#[tokio::test]
async fn from_checkout_conflict_leaves_the_worktree_clean_at_the_environment() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    let mine = old_commit(&r.checkout, "README.md", "mine");
    let moved = move_main(&r, "README.md", "theirs");
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "MERGE_CONFLICT");
    assert!(
        err.message.contains("conflict with main")
            && err.message.ends_with("Conflicting files: README.md."),
        "{}",
        err.message
    );
    let ws = r.root.path().join("work/hello-iii-iii-cloud");
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), moved);
    assert_eq!(git(&ws, &["status", "--porcelain"]), "");
    assert!(
        !std::path::Path::new(&git(&ws, &["rev-parse", "--absolute-git-dir"]))
            .join("MERGE_HEAD")
            .exists()
    );
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), mine);
    assert_eq!(git(&r.checkout, &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn from_checkout_brings_the_uncommitted_files_as_one_commit() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    for (k, v) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&r.checkout, &["config", k, v]);
    }
    git(&r.checkout, &["checkout", "-q", "-b", "feature"]);
    let mine = old_commit(&r.checkout, "one.txt", "mine");
    std::fs::write(r.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(r.checkout.join("new.txt"), "new").unwrap();
    std::fs::write(r.checkout.join(".env"), "SECRET=1").unwrap();
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    assert_eq!(git(&ws, &["rev-parse", "HEAD~1"]), mine);
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%s"]),
        "Uncommitted changes from feature"
    );
    let mut files: Vec<String> = git(&ws, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    files.sort();
    assert_eq!(files, vec!["README.md", "new.txt"], "never the .env");
    assert_eq!(
        git(&ws, &["status", "--porcelain"]),
        "",
        "nothing left over"
    );
    // The person's work, the brought commit included, is the base for the agent.
    assert_eq!(
        w.from_head.as_deref(),
        Some(git(&ws, &["rev-parse", "HEAD"]).as_str())
    );
    // The checkout itself is untouched.
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), mine);
    assert!(!git(&r.checkout, &["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn refresh_lists_the_checkouts_commits_the_environment_does_not_serve() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let served = git(&r.origin, &["rev-parse", "main"]);
    let a = old_commit(&r.checkout, "a.txt", "local a");
    let b = old_commit(&r.checkout, "b.txt", "local b");
    let refresh = || async {
        iii_cloud::status::status(
            &h.ctx,
            StatusRequest {
                refresh: Some(true),
            },
        )
        .await
        .unwrap()
        .project
        .unwrap()
        .checkout
        .not_served
    };
    assert_eq!(refresh().await, None, "nothing is served yet");
    let serve = |sha: &str| {
        let d = iii_cloud::watch::to_deploy(
            "http://x",
            (None, None, "production"),
            &serde_json::json!({"id": "d1", "number": 1, "environment_id": ENV, "type": "push", "status": "ready",
                "commit": {"sha": sha}, "stages": [], "error": null, "created_at": "2026-11-25T10:00:00Z"}),
            None,
            false,
        )
        .unwrap();
        h.ctx.store.update(|s| s.put_deploy(d));
    };
    serve(&served);
    let lines = |v: Option<Vec<CommitLine>>| {
        v.map(|v| {
            v.into_iter()
                .map(|l| (l.sha, l.message))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        lines(refresh().await),
        Some(vec![(b, "local b".into()), (a, "local a".into())])
    );
    // A served commit origin does not have: unknown, not "everything".
    serve("0123456789abcdef0123456789abcdef01234567");
    assert_eq!(refresh().await, None);
}

#[tokio::test]
async fn brought_files_never_carry_secrets_links_or_glob_matches() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    for (k, v) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&r.checkout, &["config", k, v]);
    }
    git(&r.checkout, &["checkout", "-q", "-b", "feature"]);
    std::fs::create_dir_all(r.checkout.join("config")).unwrap();
    for (f, text) in [
        (".env", "SECRET=old"),
        ("config/.env.local", "LOCAL=old"),
        ("a.txt", "a"),
        ("b.txt", "b"),
    ] {
        std::fs::write(r.checkout.join(f), text).unwrap();
    }
    old_commit(&r.checkout, "tracked.txt", "tracked files, .env included");
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    // The agent's own change, not committed yet.
    std::fs::write(ws.join("a.txt"), "agent").unwrap();
    // The person edits the tracked secrets and adds a link to a secret outside.
    std::fs::write(r.checkout.join(".env"), "SECRET=new").unwrap();
    std::fs::write(r.checkout.join("config/.env.local"), "LOCAL=new").unwrap();
    std::fs::write(r.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(r.checkout.join("[ab].txt"), "glob").unwrap();
    let secret = r.root.path().join("credentials");
    std::fs::write(&secret, "AWS_SECRET=1").unwrap();
    std::os::unix::fs::symlink(&secret, r.checkout.join("notes.md")).unwrap();
    // Keys and credential files, by name or extension: one of each kind.
    let keys = [
        "deploy.pem",
        "config/tls.key",
        "cert.p12",
        "cert.pfx",
        "id_rsa",
        "id_ed25519.pub",
        "id_ecdsa",
        ".npmrc",
        ".netrc",
        ".pypirc",
        "credentials.json",
        "release.keystore",
    ];
    for k in keys {
        std::fs::write(r.checkout.join(k), "secret").unwrap();
    }
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let mut left: Vec<String> = [".env", "config/.env.local"]
        .into_iter()
        .chain(keys)
        .map(str::to_owned)
        .collect();
    left.sort();
    assert_eq!(w.not_brought, left);
    for f in &left {
        assert_eq!(
            w.not_brought_reasons.get(f).map(String::as_str),
            Some(iii_cloud::project::POSSIBLE_SECRET),
            "{f}"
        );
        if keys.contains(&f.as_str()) {
            assert!(!ws.join(f).exists(), "{f} never enters the worktree");
        }
    }
    let mut files: Vec<String> = git(&ws, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec!["README.md", "[ab].txt", "notes.md"],
        "no secret, and `[ab]` matched only itself"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join(".env")).unwrap(),
        "SECRET=old"
    );
    assert!(
        git(&ws, &["status", "--porcelain"]).contains("M a.txt"),
        "the agent's change stays uncommitted"
    );
    // The link is a link: what it points to never enters the worktree or git.
    assert!(
        std::fs::symlink_metadata(ws.join("notes.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        git(&ws, &["show", "HEAD:notes.md"]),
        secret.to_string_lossy()
    );
    // A file in the worktree is not the same as a link to the same bytes.
    std::fs::write(ws.join("other.md"), "AWS_SECRET=1").unwrap();
    std::os::unix::fs::symlink(&secret, r.checkout.join("other.md")).unwrap();
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap_err();
    assert_eq!(err.code, "BRING_CHANGES_CONFLICT");
    assert!(err.message.contains("other.md"), "{}", err.message);
}

#[test]
fn recent_projects_keep_one_entry_per_folder_and_project_and_at_most_eight() {
    let entry = |path: &str, cloud: &str| RecentProject {
        path: path.into(),
        repository: "ada/hello-iii".into(),
        cloud_project_id: cloud.into(),
        cloud_project_name: "registry".into(),
        team_name: None,
        environment_id: None,
        environment_name: None,
        mode: WorkspaceMode::Worktree,
        used_at: "t".into(),
        team_id: None,
    };
    let mut s = iii_cloud::store::State::default();
    for i in 0..10 {
        s.remember_project(entry(&format!("/r{i}"), "p"));
    }
    let paths: Vec<&str> = s.recent_projects.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/r9", "/r8", "/r7", "/r6", "/r5", "/r4", "/r3", "/r2"]
    );
    // Used again: moved to the front, not listed twice. Another cloud project
    // of the same folder is its own entry.
    s.remember_project(entry("/r5", "p"));
    s.remember_project(entry("/r9", "other"));
    let keys: Vec<(&str, &str)> = s
        .recent_projects
        .iter()
        .map(|r| (r.path.as_str(), r.cloud_project_id.as_str()))
        .collect();
    assert_eq!(&keys[..3], &[("/r9", "other"), ("/r5", "p"), ("/r9", "p")]);
    assert_eq!(keys.len(), 8);
    assert_eq!(keys.iter().filter(|k| k.0 == "/r5").count(), 1);
}

#[tokio::test]
async fn an_api_key_session_keeps_no_recent_project() {
    let server = MockServer::start().await;
    let (r1, r2) = (repo(), repo());
    let h = connected(&server, "worker-compose.yaml").await;
    use_project(&h.ctx, use_req(&r1.checkout, false))
        .await
        .unwrap();
    use_project(&h.ctx, use_req(&r2.checkout, false))
        .await
        .unwrap();
    // A key connects one environment and has no cloud project to switch to.
    assert!(h.ctx.store.read(|s| s.recent_projects.is_empty()));
}

/// A branch `name` on origin only, one commit `file` past origin's main.
fn origin_branch(r: &Repo, name: &str, file: &str) -> String {
    let t = r.root.path().join(format!("clone-{name}"));
    git(
        r.root.path(),
        &[
            "clone",
            "-q",
            r.origin.to_str().unwrap(),
            t.file_name().unwrap().to_str().unwrap(),
        ],
    );
    git(&t, &["checkout", "-q", "-b", name]);
    std::fs::write(t.join(file), name).unwrap();
    git(&t, &["add", "."]);
    git(&t, &["commit", "-m", name]);
    git(&t, &["push", "-q", "origin", name]);
    git(&t, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn refresh_lists_the_branches_newest_first_with_what_the_environment_lacks() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    feature(&r);
    // 36 old branches: only the newest 30 branches are listed.
    git(&r.checkout, &["checkout", "-q", "-b", "old"]);
    old_commit(&r.checkout, "old.txt", "old");
    for i in 0..35 {
        git(&r.checkout, &["branch", &format!("b{i}")]);
    }
    git(&r.checkout, &["checkout", "-q", "feature"]);
    for f in ["one.txt", "two.txt"] {
        std::fs::write(r.checkout.join(f), f).unwrap();
        git(&r.checkout, &["add", "."]);
        git(&r.checkout, &["commit", "-q", "-m", f]);
    }
    origin_branch(&r, "feature-x", "x.txt");
    use_project(&h.ctx, use_req(&r.checkout, false))
        .await
        .unwrap();
    let branches = iii_cloud::status::status(
        &h.ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await
    .unwrap()
    .project
    .unwrap()
    .checkout
    .branches;
    assert_eq!(branches.len(), 30);
    let find = |n: &str| branches.iter().find(|b| b.name == n);
    let f = find("feature").unwrap();
    assert_eq!(
        (f.remote, f.ahead, f.is_env_branch),
        (false, Some(2), false)
    );
    let main = find("main").unwrap();
    assert_eq!((main.ahead, main.is_env_branch), (Some(0), true));
    assert!(
        find("origin/main").is_none(),
        "the local main stands for it"
    );
    let x = find("origin/feature-x").unwrap();
    assert_eq!((x.remote, x.ahead), (true, Some(1)));
    assert!(
        branches
            .windows(2)
            .all(|w| w[0].committed_at >= w[1].committed_at),
        "newest first"
    );
    assert_eq!(f.sha, git(&r.checkout, &["rev-parse", "feature"]));
}

#[tokio::test]
async fn from_checkout_takes_the_branch_named_as_source_instead_of_head() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    let mine = old_commit(&r.checkout, "one.txt", "mine");
    let with = |source: &str, bring: bool| UseRequest {
        source: Some(source.into()),
        ..from_checkout(&r.checkout, ENV2, bring)
    };
    // The environment's own branch: nothing of the person's comes in.
    let w = use_project(&h.ctx, with("main", false))
        .await
        .unwrap()
        .workspace
        .unwrap();
    let ws = std::path::PathBuf::from(&w.path);
    let main = git(&r.origin, &["rev-parse", "main"]);
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), main);
    assert!(!ws.join("one.txt").exists());
    assert_eq!(
        (w.source.as_deref(), w.from_head.as_deref()),
        (Some("main"), Some(main.as_str()))
    );
    // Another branch, on origin only, diverged from main: merged in the
    // worktree; the folder's changed file still comes on top.
    let x = origin_branch(&r, "feature-x", "x.txt");
    let moved = move_main(&r, "m.txt", "m");
    std::fs::write(r.checkout.join("new.txt"), "new").unwrap();
    let w = use_project(&h.ctx, with("origin/feature-x", true))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%s", "HEAD~1"]),
        "Merge origin/feature-x into main for the check"
    );
    assert_eq!(
        git(&ws, &["log", "-1", "--format=%P", "HEAD~1"]),
        format!("{moved} {x}")
    );
    assert_eq!(
        git(&ws, &["show", "--name-only", "--format=", "HEAD"]),
        "new.txt"
    );
    assert_eq!(w.source.as_deref(), Some("origin/feature-x"));
    assert_eq!(
        w.from_head.as_deref(),
        Some(git(&ws, &["rev-parse", "HEAD"]).as_str())
    );
    // A plain `use` keeps the source with what the worktree took.
    let plain = use_project(
        &h.ctx,
        UseRequest {
            environment_id: Some(ENV2.into()),
            ..use_req(&r.checkout, false)
        },
    )
    .await
    .unwrap()
    .workspace
    .unwrap();
    assert_eq!(plain.source.as_deref(), Some("origin/feature-x"));
    // The checkout is never touched.
    assert_eq!(git(&r.checkout, &["rev-parse", "HEAD"]), mine);
    assert_eq!(
        git(&r.checkout, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature"
    );
    assert_eq!(git(&r.checkout, &["status", "--porcelain"]), "?? new.txt");
    // A branch that is not here, or a source without `from: checkout`, is refused.
    for req in [
        with("nope", false),
        with("origin/nope", false),
        UseRequest {
            source: Some("main".into()),
            environment_id: Some(ENV2.into()),
            ..use_req(&r.checkout, false)
        },
    ] {
        let err = use_project(&h.ctx, req).await.unwrap_err();
        assert_eq!(err.code, "INVALID_REQUEST", "{}", err.message);
    }
}

#[tokio::test]
async fn a_merge_the_agent_has_not_committed_is_never_touched() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    old_commit(&r.checkout, "README.md", "mine");
    move_main(&r, "README.md", "theirs");
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "MERGE_CONFLICT");
    // The agent merges by hand, resolves and stages, and has not committed yet.
    let ws = r.root.path().join("work/hello-iii-iii-cloud");
    let merge = std::process::Command::new("git")
        .args(["merge", "feature"])
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(!merge.status.success(), "a conflict");
    std::fs::write(ws.join("README.md"), "both").unwrap();
    git(&ws, &["add", "README.md"]);
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap_err();
    assert_eq!(err.code, "MERGE_IN_PROGRESS");
    assert!(
        err.message
            .starts_with("Your agent started merging feature in "),
        "{}",
        err.message
    );
    git(&ws, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    assert_eq!(git(&ws, &["diff", "--cached", "--name-only"]), "README.md");
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "both"
    );
}

#[tokio::test]
async fn the_fix_branch_out_in_the_persons_checkout_gets_a_new_name_and_the_checkout_untouched() {
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    git(
        &r.checkout,
        &["checkout", "-q", "-b", "iii-cloud/ready-staging"],
    );
    old_commit(&r.checkout, "one.txt", "mine");
    std::fs::write(r.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(r.checkout.join("new.txt"), "new").unwrap();
    let before = (
        git(&r.checkout, &["rev-parse", "HEAD"]),
        git(&r.checkout, &["status", "--porcelain"]),
    );
    let w = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap()
        .workspace
        .unwrap();
    assert_eq!(w.branch, "iii-cloud/ready-staging-2");
    let ws = std::path::Path::new(&w.path);
    assert!(!iii_cloud::project::same_path(ws, &r.checkout));
    assert_eq!(std::fs::read_to_string(ws.join("new.txt")).unwrap(), "new");
    assert_eq!(
        (
            git(&r.checkout, &["rev-parse", "HEAD"]),
            git(&r.checkout, &["status", "--porcelain"]),
        ),
        before
    );
    assert_eq!(
        std::fs::read_to_string(r.checkout.join("README.md")).unwrap(),
        "changed\n"
    );
    assert_eq!(
        std::fs::read_to_string(r.checkout.join("new.txt")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn a_bring_that_fails_half_way_leaves_the_worktree_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let server = MockServer::start().await;
    let r = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    staging(&server).await;
    feature(&r);
    let ws = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, false))
        .await
        .unwrap()
        .workspace
        .unwrap()
        .path;
    let ws = std::path::PathBuf::from(ws);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    // A tracked change applies, a readable new file copies, then one can't be read.
    std::fs::write(r.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(r.checkout.join("a.txt"), "a").unwrap();
    let locked = r.checkout.join("z.txt");
    std::fs::write(&locked, "z").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        // root reads anything: nothing to fail here.
        return;
    }
    let err = use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap_err();
    assert_eq!(err.code, "GIT_FAILED", "{}", err.message);
    assert_eq!(git(&ws, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&ws, &["status", "--porcelain"]),
        "",
        "nothing half-brought"
    );
    // Readable again: the same bring works.
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    use_project(&h.ctx, from_checkout(&r.checkout, ENV2, true))
        .await
        .unwrap();
    let mut files: Vec<String> = git(&ws, &["show", "--name-only", "--format=", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect();
    files.sort();
    assert_eq!(files, vec!["README.md", "a.txt", "z.txt"]);
    assert_eq!(git(&ws, &["status", "--porcelain"]), "");
}
